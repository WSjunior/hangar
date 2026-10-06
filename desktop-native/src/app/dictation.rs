use super::*;
use std::{io::Read, sync::Mutex};

const STYLES: [&str; 3] = ["limpar", "prosa", "briefing"];
const PCM_LIMIT: usize = 16_000 * 2 * 180;
const SILENCE: Duration = Duration::from_secs(2);
const COUNTDOWN: Duration = Duration::from_secs(3);

#[derive(Default)]
struct Vad {
    peak: f32,
    last: Option<Instant>,
    quiet_since: Option<Instant>,
}

impl Vad {
    fn step(&mut self, rms: f32, now: Instant) -> bool {
        let elapsed = self.last.map(|last| now.duration_since(last).as_secs_f32() * 1000.).unwrap_or(0.);
        self.last = Some(now);
        let decayed = self.peak * 0.98_f32.powf(elapsed / 55.);
        self.peak = if rms > decayed { decayed + (rms - decayed) * 0.08 } else { decayed };
        if self.peak <= 0.01 || rms >= self.peak * 0.25 {
            self.quiet_since = None;
            return false;
        }
        let since = *self.quiet_since.get_or_insert(now);
        now.duration_since(since) >= SILENCE
    }
}

/// Junta os canais num só e reduz para 16 kHz pela média de cada janela: o formato que o backend transcreve.
struct Downmix {
    channels: usize,
    step: f64,
    phase: f64,
    sum: f32,
    count: u32,
    last: f32,
}

impl Downmix {
    fn new(channels: u16, rate: u32) -> Self {
        Self { channels: channels.max(1) as usize, step: rate as f64 / 16_000., phase: 0., sum: 0., count: 0, last: 0. }
    }

    fn push<T: Copy>(&mut self, data: &[T], sample: impl Fn(T) -> f32, out: &mut Vec<u8>) {
        for frame in data.chunks(self.channels) {
            let mono = frame.iter().map(|value| sample(*value)).sum::<f32>() / frame.len() as f32;
            self.sum += mono;
            self.count += 1;
            self.phase += 1.;
            while self.phase >= self.step {
                self.phase -= self.step;
                if self.count > 0 { self.last = self.sum / self.count as f32; self.sum = 0.; self.count = 0; }
                if out.len() + 2 > PCM_LIMIT { return; }
                out.extend_from_slice(&((self.last.clamp(-1., 1.) * 32767.) as i16).to_le_bytes());
            }
        }
    }
}

struct Recorder {
    stream: Option<cpal::Stream>,
    failed: Arc<std::sync::atomic::AtomicBool>,
    pcm: Arc<Mutex<Vec<u8>>>,
    playback: Option<Instant>,
    sampled: usize,
    last_signal: (f32, f32),
    last_pcm_at: Option<Instant>,
}

impl Recorder {
    fn start() -> Result<Self, Failure> {
        let mut recorder = Self { stream: None, failed: Default::default(), pcm: Default::default(), playback: None,
            sampled: 0, last_signal: (0., 0.), last_pcm_at: None };
        if let Some(path) = std::env::var_os("HANGAR_NATIVE_DICTATION_WAV") {
            let mut bytes = Vec::new();
            std::fs::File::open(path).and_then(|file| file.take((PCM_LIMIT + 4097) as u64).read_to_end(&mut bytes))
                .map_err(|_| Failure::local("dictation_wav_error"))?;
            *recorder.pcm.lock().unwrap() = wav_pcm(&bytes).ok_or_else(|| Failure::local("dictation_wav_error"))?.to_vec();
            recorder.playback = Some(Instant::now());
            return Ok(recorder);
        }
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let device = cpal::default_host().default_input_device().ok_or_else(|| Failure::local("dictation_no_microphone"))?;
        let config = device.default_input_config().map_err(|error| {
            eprintln!("dictation config: {error}");
            Failure::local("dictation_recorder_error")
        })?;
        // Reserva os 180 s de uma vez: crescer o Vec dentro da função de áudio copiaria megabytes em tempo real.
        *recorder.pcm.lock().unwrap() = Vec::with_capacity(PCM_LIMIT);
        let (pcm, failed) = (recorder.pcm.clone(), recorder.failed.clone());
        let mut mix = Downmix::new(config.channels(), config.sample_rate());
        let on_error = move |error: cpal::Error| {
            eprintln!("dictation stream: {error}");
            // Estouro de buffer perde um pedaço e segue; o resto é microfone sumido ou captura parada.
            if !matches!(error.kind(), cpal::ErrorKind::Xrun | cpal::ErrorKind::RealtimeDenied) {
                failed.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        };
        macro_rules! input {
            ($t:ty, $to_f32:expr) => {
                device.build_input_stream::<$t, _, _>(config.clone().into(),
                    move |data, _| mix.push(data, $to_f32, &mut pcm.lock().unwrap()), on_error, None)
            };
        }
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => input!(f32, |v: f32| v),
            cpal::SampleFormat::I16 => input!(i16, |v: i16| v as f32 / 32768.),
            cpal::SampleFormat::I32 => input!(i32, |v: i32| v as f32 / 2_147_483_648.),
            cpal::SampleFormat::U16 => input!(u16, |v: u16| (v as f32 - 32768.) / 32768.),
            cpal::SampleFormat::U8 => input!(u8, |v: u8| (v as f32 - 128.) / 128.),
            cpal::SampleFormat::I8 => input!(i8, |v: i8| v as f32 / 128.),
            cpal::SampleFormat::F64 => input!(f64, |v: f64| v as f32),
            other => {
                eprintln!("dictation format: {other:?}");
                return Err(Failure::local("dictation_recorder_error"));
            }
        }.map_err(|error| {
            eprintln!("dictation open: {error}");
            Failure::local("dictation_recorder_error")
        })?;
        stream.play().map_err(|error| {
            eprintln!("dictation play: {error}");
            Failure::local("dictation_recorder_error")
        })?;
        recorder.stream = Some(stream);
        Ok(recorder)
    }

    fn failed(&self) -> bool { self.failed.load(std::sync::atomic::Ordering::Relaxed) }

    fn signal(&mut self) -> (f32, f32) {
        let bytes = self.pcm.lock().unwrap();
        let end = self.playback.map(|start| (start.elapsed().as_millis() as usize * 32).min(bytes.len())).unwrap_or(bytes.len()) & !1;
        let start = self.sampled.min(end);
        self.sampled = end;
        // O sistema entrega blocos; um intervalo sem bloco ainda é áudio recente, mas uma captura travada não é fala eterna.
        if start == end {
            return if self.last_pcm_at.is_some_and(|at| at.elapsed() < Duration::from_millis(4096 / 32 + 55)) {
                self.last_signal
            } else { (0., 0.) };
        }
        let mut peak: f32 = 0.;
        let mut sum = 0.;
        let mut count = 0;
        for sample in bytes[start..end].chunks_exact(2) {
            let value = i16::from_le_bytes([sample[0], sample[1]]) as f32 / 32768.;
            peak = peak.max(value.abs());
            sum += value * value;
            count += 1;
        }
        self.last_signal = (peak, if count == 0 { 0. } else { (sum / count as f32).sqrt() });
        self.last_pcm_at = Some(Instant::now());
        self.last_signal
    }

    fn finish(mut self) -> Result<Vec<u8>, Failure> {
        drop(self.stream.take());
        let pcm = self.pcm.lock().unwrap();
        // Só zeros é captura muda (permissão negada no macOS entrega silêncio): o Whisper inventaria texto.
        if pcm.len() < 2 || pcm.iter().all(|byte| *byte == 0) { return Err(Failure::local("dictation_empty_audio")); }
        Ok(wav(&pcm[..pcm.len() & !1]))
    }
}

fn wav(pcm: &[u8]) -> Vec<u8> {
    let mut bytes = b"RIFF".to_vec();
    bytes.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\x3e\0\0\0\x7d\0\0\x02\0\x10\0data");
    bytes.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    bytes.extend_from_slice(pcm);
    bytes
}

fn wav_pcm(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() > PCM_LIMIT + 4096 || bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" { return None; }
    let (mut offset, mut format, mut data) = (12usize, false, None);
    while offset + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
        let chunk = bytes.get(offset + 8..offset.checked_add(8)?.checked_add(size)?)?;
        match &bytes[offset..offset + 4] {
            b"fmt " => format = chunk.get(..16) == Some(&b"\x01\0\x01\0\x80\x3e\0\0\0\x7d\0\0\x02\0\x10\0"[..]),
            b"data" => data = Some(chunk), _ => {}
        }
        offset += 8 + size + (size % 2);
    }
    data.filter(|pcm| format && !pcm.is_empty() && pcm.len() <= PCM_LIMIT && pcm.len() % 2 == 0)
}

#[derive(Default)]
pub(super) struct Dictation {
    seq: u64,
    owner: Option<SessionOwner>,
    recorder: Option<Recorder>,
    request: Option<JoinHandle<()>>,
    started: Option<Instant>,
    level: f32,
    /// Nível de cada passo da gravação, o mais novo no fim: vira a onda que desliza, como no web.
    bars: std::collections::VecDeque<f32>,
    result: Option<Value>,
    style: Option<(u64, &'static str)>,
    style_writes: u64,
    style_task: Option<Task<()>>,
    audio: Arc<Mutex<Vec<u8>>>,
    file_name: Option<String>,
    file_owner: Option<SessionOwner>,
    file_generation: u64,
    versions: HashMap<String, Value>,
    inserted: Option<(String, std::ops::Range<usize>)>,
    cleaning: bool,
    error: Option<String>,
    hands_free: bool,
    auto_send: bool,
    timed_out: bool,
    vad: Vad,
    countdown: Option<Instant>,
}

impl Dictation {
    fn observe_file_owner(&mut self, owner: Option<SessionOwner>) -> u64 {
        if self.file_owner != owner {
            self.file_owner = owner;
            self.file_generation += 1;
        }
        self.file_generation
    }

    fn style(&self, connection: u64) -> Option<&'static str> {
        self.style.filter(|(owner, _)| *owner == connection).map(|(_, style)| style)
    }

    fn text_in_field(&self, value: &str) -> bool {
        self.inserted.as_ref().is_some_and(|(draft, range)| value.get(range.clone()) == draft.get(range.clone()))
    }

    fn draft_matches(&self, value: &str) -> bool {
        self.inserted.as_ref().is_none_or(|(draft, _)| draft == value)
    }

    fn cancel(&mut self) {
        self.seq += 1;
        self.owner = None;
        self.recorder = None;
        if let Some(task) = self.request.take() { task.abort(); }
        self.started = None;
        self.level = 0.;
        self.bars.clear();
        self.result = None;
        self.audio = Default::default();
        self.file_name = None;
        self.versions.clear();
        self.inserted = None;
        self.cleaning = false;
        self.error = None;
        self.hands_free = false;
        self.auto_send = false;
        self.timed_out = false;
        self.vad = Vad::default();
        self.countdown = None;
    }
}

impl Drop for Dictation { fn drop(&mut self) { self.cancel(); } }

impl Hangar {
    /// Dono do ditado: a sessão aberta ou, sem ela, a tela sem sessão (nome vazio) antes do Enviar.
    pub(super) fn dictation_owner(&self, cx: &App) -> Option<SessionOwner> {
        self.session_owner().or_else(|| {
            if !self.new_chat_screen() || self.opening.is_some() { return None; }
            Some((self.connection, self.new_chat_api(cx)?.identity(), String::new()))
        })
    }

    pub(super) fn check_dictation_owner(&mut self, cx: &mut Context<Self>) -> u64 {
        let owner = self.dictation_owner(cx);
        let generation = self.dictation.observe_file_owner(owner.clone());
        if self.dictation.owner.is_some() && self.dictation.owner != owner {
            self.cancel_dictation();
            self.redraw(panes::Area::Bottom, cx);
        }
        generation
    }

    /// Cancelar solta a gravação guardada: o player dela para junto.
    fn cancel_dictation(&mut self) {
        self.dictation.cancel();
        self.stop_audio("dictation");
    }

    fn dictation_ready(&self) -> bool {
        if self.selected.is_none() { return self.new_chat_screen() && self.opening.is_none(); }
        self.selected_key().is_some() && self.chat_online && self.history_installed
    }

    /// Para onde vai o áudio: a sessão aberta, ou a máquina escolhida nos chips da tela sem sessão, sem sessão ainda.
    fn dictation_target(&self, cx: &App) -> Option<(Api, Option<String>)> {
        match self.selected_key() {
            Some(key) => Some((self.session_api()?, Some(key.name))),
            None => Some((self.new_chat_api(cx)?, None)),
        }
    }

    pub(super) fn watch_dictation(_window: &Window, cx: &mut Context<Self>) {
        let mut style_connection = None;
        cx.observe_self(move |this, cx| {
            this.check_dictation_owner(cx);
            if style_connection != Some(this.connection) {
                style_connection = Some(this.connection);
                this.dictation.style_task = None;
            }
            if this.api.is_some() && this.dictation.style(this.connection).is_none() && this.dictation.style_task.is_none() {
                this.load_dictation_style(cx);
            }
        }).detach();
        let owner = cx.entity().downgrade();
        cx.intercept_keystrokes(move |_, _, cx| {
            let _ = owner.update(cx, |this, cx| this.cancel_dictation_countdown(cx));
        }).detach();
    }

    fn cancel_dictation_countdown(&mut self, cx: &mut Context<Self>) {
        if self.dictation.countdown.take().is_some() {
            self.redraw(panes::Area::Bottom, cx);
            cx.notify();
        }
    }

    fn load_dictation_style(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else { return; };
        let (connection, writes) = (self.connection, self.dictation.style_writes);
        let job = self.runtime.spawn(async move { api.config().await });
        self.dictation.style_task = Some(cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                if this.connection != connection || this.dictation.style_writes != writes { return; }
                if let Ok(Ok(value)) = result {
                    if let Some(style) = STYLES.into_iter().find(|style| value.pointer("/campos/ditado_estilo/valor").and_then(Value::as_str) == Some(*style)) {
                        this.dictation.style = Some((connection, style));
                        cx.notify();
                    }
                }
            });
        }));
    }

    fn set_dictation_style(&mut self, style: &'static str, cx: &mut Context<Self>) {
        if self.dictation.recorder.is_some() || self.dictation.request.is_some() { return; }
        let Some(api) = self.api.clone() else { return; };
        let (connection, before) = (self.connection, self.dictation.style);
        self.dictation.style = Some((connection, style));
        self.dictation.style_writes += 1;
        self.dictation.error = None;
        let mine = self.dictation.style_writes;
        let job = self.runtime.spawn(async move {
            api.server_send(reqwest::Method::POST, &["config"], Some(json!({"ditado_estilo": style})), 8).await
        });
        cx.spawn(async move |this, cx| {
            let result = job.await.unwrap_or_else(|_| Err(Failure::local("invalid_response")));
            let _ = this.update(cx, |this, cx| {
                if this.connection != connection || this.dictation.style_writes != mine { return; }
                if let Err(error) = result {
                    this.dictation.style = before;
                    this.dictation.error = Some(Self::failure(&error));
                    cx.notify();
                }
            });
        }).detach();
        cx.notify();
    }

    pub(super) fn toggle_dictation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_dictation_countdown(cx);
        if self.dictation.request.is_some() { return; }
        if self.dictation.recorder.is_some() {
            self.stop_dictation(false, false, cx);
            self.composer.update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        if self.connection_dialog || self.settings.is_some() || window.has_active_dialog(cx) { return; }
        if !self.dictation_ready() { return; }
        self.cancel_dictation();
        match Recorder::start() {
            Ok(recorder) => {
                self.dictation.owner = self.dictation_owner(cx);
                self.dictation.hands_free = appearance::get().hands_free;
                self.dictation.recorder = Some(recorder);
                self.dictation.started = Some(Instant::now());
                let seq = self.dictation.seq;
                cx.spawn_in(window, async move |this, cx| {
                    loop {
                        cx.background_executor().timer(Duration::from_millis(55)).await;
                        let keep = this.update_in(cx, |this, window, cx| {
                            if this.dictation.seq != seq { return false; }
                            let Some(recorder) = &mut this.dictation.recorder else { return false; };
                            if recorder.failed() {
                                // Microfone caiu no meio (headset trocado): transcreve o que já foi gravado.
                                if recorder.pcm.lock().unwrap().len() >= 2 {
                                    this.stop_dictation(false, false, cx);
                                } else {
                                    this.cancel_dictation();
                                    window.push_notification(Notification::error(tr("dictation_recorder_error")), cx);
                                }
                                this.redraw(panes::Area::Bottom, cx);
                                return false;
                            }
                            let (level, rms) = recorder.signal();
                            this.dictation.level = level;
                            // Teto de 320, o mesmo do web: cobre a faixa inteira numa janela larga.
                            if this.dictation.bars.len() >= 320 { this.dictation.bars.pop_front(); }
                            // RMS ×5 como no web: voz normal fica em 0,05–0,2 e sem ganho a onda mal sai do chão.
                            this.dictation.bars.push_back((rms * 5.).min(1.));
                            if this.dictation.hands_free && this.dictation.vad.step(rms, Instant::now()) {
                                this.stop_dictation(true, false, cx);
                                this.redraw(panes::Area::Bottom, cx);
                                return false;
                            }
                            this.redraw(panes::Area::Bottom, cx);
                            if this.dictation.started.is_some_and(|start| start.elapsed() >= Duration::from_secs(180)) {
                                this.stop_dictation(false, this.dictation.hands_free, cx);
                            }
                            true
                        });
                        if !matches!(keep, Ok(true)) { break; }
                    }
                }).detach();
            }
            Err(error) => window.push_notification(Notification::error(Self::dictation_failure(&error)), cx),
        }
        cx.notify();
    }

    pub(super) fn transcribe_file(&mut self, key: &SessionKey, filename: String, bytes: Vec<u8>, cx: &mut Context<Self>) -> Result<(), String> {
        if bytes.len() as u64 > api::MAX_BYTES { return Err(tr("attach_too_big_named").replace("{name}", &filename)); }
        if !self.dictation_ready() || self.composer_key().as_ref() != Some(key) { return Err(tr("connection_failed")); }
        let Some(owner) = self.dictation_owner(cx) else { return Err(tr("connection_failed")); };
        if self.dictation.recorder.is_some() || self.dictation.request.is_some() {
            return Err(tr_shared("composer_aguarde_transcricao", &[]));
        }
        let Some((api, session)) = self.dictation_target(cx) else { return Err(tr("connection_failed")); };
        self.cancel_dictation();
        self.dictation.owner = Some(owner);
        self.dictation.file_name = Some(filename.clone());
        *self.dictation.audio.lock().unwrap() = bytes.clone();
        let (tx, connection, seq) = (self.tx.clone(), self.connection, self.dictation.seq);
        self.dictation.request = Some(self.runtime.spawn(async move {
            let result = api.transcribe(session.as_deref(), &filename, bytes, false, None).await;
            let _ = tx.send(Envelope { connection, selection: None, payload: Payload::Dictation(seq, result) }).await;
        }));
        cx.notify();
        Ok(())
    }

    fn stop_dictation(&mut self, silence: bool, timed_out: bool, cx: &mut Context<Self>) {
        let Some(recorder) = self.dictation.recorder.take() else { return; };
        let Some((api, session)) = self.dictation_target(cx) else { self.cancel_dictation(); return; };
        self.dictation.auto_send = silence && self.dictation.hands_free;
        self.dictation.timed_out = timed_out;
        let audio_cache = self.dictation.audio.clone();
        let style = self.dictation.style(self.connection);
        let (tx, connection, seq) = (self.tx.clone(), self.connection, self.dictation.seq);
        // O fluxo de áudio não troca de thread; soltá-lo e montar o WAV é rápido o bastante para a tela.
        let audio = recorder.finish();
        self.dictation.request = Some(self.runtime.spawn(async move {
            let result = match audio {
                Ok(bytes) => {
                    *audio_cache.lock().unwrap() = bytes.clone();
                    api.transcribe(session.as_deref(), "ditado.wav", bytes, true, style).await
                },
                Err(error) => Err(error),
            };
            let _ = tx.send(Envelope { connection, selection: None, payload: Payload::Dictation(seq, result) }).await;
        }));
        cx.notify();
    }

    fn start_dictation_countdown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let deadline = Instant::now() + COUNTDOWN;
        let seq = self.dictation.seq;
        self.dictation.countdown = Some(deadline);
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let keep = this.update_in(cx, |this, window, cx| {
                    if this.dictation.seq != seq || this.dictation.countdown != Some(deadline)
                        || this.dictation.owner.is_none() || this.dictation.owner != this.dictation_owner(cx) { return false; }
                    if Instant::now() < deadline {
                        this.redraw(panes::Area::Bottom, cx);
                        cx.notify();
                        return true;
                    }
                    this.dictation.countdown = None;
                    let key = this.selected_key();
                    // Sem sessão, enviar é criar: deu certo quando a abertura começou.
                    if this.selected.is_none() {
                        this.submit(false, false, window, cx);
                        if this.opening.is_none() { this.dictation.error = Some(tr("dictation_auto_send_failed")); }
                    } else if !this.can_send() || key.as_ref().is_none_or(|key| this.delivery.pending(key)
                        || this.uploading.contains_key(key) || this.attachments.get(key).is_some_and(|files| !files.is_empty())) {
                        this.dictation.error = Some(tr("dictation_auto_send_failed"));
                    } else {
                        this.submit(false, false, window, cx);
                        if key.as_ref().is_some_and(|key| !this.delivery.pending(key)) {
                            this.dictation.error = Some(tr("dictation_auto_send_failed"));
                        }
                    }
                    this.redraw(panes::Area::Bottom, cx);
                    cx.notify();
                    false
                });
                if !matches!(keep, Ok(true)) { break; }
            }
        }).detach();
        self.redraw(panes::Area::Bottom, cx);
        cx.notify();
    }

    fn revise_dictation(&mut self, style: Option<&'static str>, window: &mut Window, cx: &mut Context<Self>) {
        if self.dictation.request.is_some() || self.dictation.recorder.is_some()
            || self.dictation.owner.is_none() || self.dictation.owner != self.dictation_owner(cx)
            || (self.dictation.file_name.is_some() && style.is_some()) { return; }
        if !self.dictation.draft_matches(&self.composer.read(cx).value()) {
            self.dictation.error = Some(tr("dictation_draft_changed"));
            cx.notify();
            return;
        }
        self.dictation.error = None;
        if let Some(value) = style.and_then(|style| self.dictation.versions.get(style)).cloned() {
            self.receive_dictation(self.dictation.seq, Ok(value), window, cx);
            return;
        }
        let Some((api, session)) = self.dictation_target(cx) else { return; };
        let raw =self.dictation.result.as_ref().and_then(|v| v.get("raw")).and_then(Value::as_str).unwrap_or("").to_owned();
        let audio = self.dictation.audio.lock().unwrap().clone();
        if (style.is_some() && raw.is_empty()) || (style.is_none() && audio.is_empty()) { return; }
        self.dictation.seq += 1;
        self.dictation.cleaning = style.is_some();
        let clean = self.dictation.file_name.is_none();
        let filename = self.dictation.file_name.clone().unwrap_or_else(|| "ditado.wav".into());
        let recording_style = if clean { self.dictation.style(self.connection) } else { None };
        let (tx, connection, seq) = (self.tx.clone(), self.connection, self.dictation.seq);
        self.dictation.request = Some(self.runtime.spawn(async move {
            let result = if let Some(style) = style {
                api.server_send(reqwest::Method::POST, &["ditado", "relimpar"], Some(json!({"texto": raw, "estilo": style})), 180).await
                    .and_then(|mut value| {
                        let fields = value.as_object_mut().ok_or_else(|| Failure::local("invalid_response"))?;
                        fields.insert("raw".into(), json!(raw));
                        Ok(value)
                    })
            } else { api.transcribe(session.as_deref(), &filename, audio, clean, recording_style).await };
            let _ = tx.send(Envelope { connection, selection: None, payload: Payload::Dictation(seq, result) }).await;
        }));
        cx.notify();
    }

    pub(super) fn receive_dictation(&mut self, seq: u64, result: Result<Value, Failure>, window: &mut Window, cx: &mut Context<Self>) {
        if self.dictation.seq != seq || self.dictation.owner.is_none() || self.dictation.owner != self.dictation_owner(cx) { return; }
        let auto_send = std::mem::take(&mut self.dictation.auto_send);
        let timed_out = std::mem::take(&mut self.dictation.timed_out);
        self.dictation.request = None;
        self.dictation.started = None;
        self.dictation.level = 0.;
        self.dictation.bars.clear();
        self.dictation.cleaning = false;
        self.dictation.error = None;
        match result {
            Ok(value) => {
                let text = value.get("text").and_then(Value::as_str).unwrap_or("").trim();
                let raw = value.get("raw").and_then(Value::as_str).unwrap_or(text);
                let applied = value.get("estilo_aplicado").and_then(Value::as_str).unwrap_or("cru");
                if text.is_empty() {
                    self.dictation.error = Some(if self.dictation.file_name.is_some() {
                        tr_shared("composer_transcricao_vazia", &[])
                    } else { tr("dictation_empty_text") });
                } else if !self.dictation.draft_matches(&self.composer.read(cx).value()) {
                    self.dictation.error = Some(tr("dictation_draft_changed"));
                } else {
                    let draft_still_empty = self.composer.read(cx).value().trim().is_empty()
                        && self.composer_key().is_some_and(|key| self.attachments.get(&key).is_none_or(Vec::is_empty));
                    let previous = self.dictation.inserted.as_ref().map(|(_, range)| range.clone());
                    let inserted = self.composer.update(cx, |input, cx| {
                        let draft = input.value().to_string();
                        let range = previous.unwrap_or_else(|| input.selected_range());
                        let replacement = dictation_insert(&draft, range.clone(), text);
                        input.set_selected_range(range.clone(), cx);
                        input.replace(replacement.clone(), window, cx);
                        (input.value().to_string(), range.start..range.start + replacement.len())
                    });
                    self.dictation.inserted = Some(inserted);
                    if self.dictation.file_name.is_none() {
                        if self.dictation.result.as_ref().and_then(|v| v.get("raw")) != value.get("raw") {
                            self.dictation.versions.clear();
                        }
                        self.dictation.versions.insert("cru".into(), json!({"text": raw, "raw": raw, "estilo_aplicado": "cru"}));
                        self.dictation.versions.insert(applied.to_owned(), value.clone());
                    }
                    // Consome a mudança programática antes do observador de @menção.
                    self.refresh_mention(cx);
                    self.mention.close();
                    if let Some(warning) = value.get("aviso").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                        window.push_notification(Notification::warning(warning.to_owned()), cx);
                    }
                    let warning = value.get("aviso").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
                    self.dictation.result = Some(value);
                    if timed_out && !warning { self.dictation.error = Some(tr("dictation_silence_timeout")); }
                    if auto_send && draft_still_empty && !warning { self.start_dictation_countdown(window, cx); }
                }
            }
            Err(error) => self.dictation.error = Some(Self::dictation_failure(&error)),
        }
        cx.notify();
    }

    /// O microfone, a pílula do estilo (ao lado dele, como no web) e a faixa de estado do ditado, só quando há o que mostrar.
    pub(super) fn render_dictation(&self, readable: bool, cx: &mut Context<Self>) -> (Button, Option<AnyElement>, Option<AnyElement>) {
        let recording = self.dictation.recorder.is_some();
        let transcribing = self.dictation.request.is_some();
        let label = tr(if recording { "dictation_stop" } else if transcribing { "dictation_working" } else { "dictation_start" });
        let mic = if recording {
            Button::new("dictation-toggle").ghost().size_7().rounded_md()
                .child(div().size_3().rounded_sm().bg(theme::danger()))
        } else { chrome::icon_button("dictation-toggle", IconName::Mic, label.clone(), cx) };
        let mic = mic.accessibility_label(label.clone())
            .disabled(transcribing || (!recording && (!readable || !self.dictation_ready())))
            .loading(transcribing)
            .tooltip(format!("{label} · {}", tr("dictation_shortcut")))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_dictation(window, cx)));
        let owner = self.dictation.owner.is_some() && self.dictation.owner == self.dictation_owner(cx);
        let style = self.dictation.style(self.connection).unwrap_or("prosa");
        let entity = cx.entity().downgrade();
        // Gravando, some: trocar no meio não muda nada (o backend lê o estilo no fim) e o espaço é do botão de parar.
        let pill = (!recording).then(|| chrome::pill_button("dictation-style", cx).pl(px(10.)).gap(px(6.))
            .tooltip(tr("dictation_style")).accessibility_label(format!("{}: {}", tr("dictation_style"), style_label(style)))
            .disabled(transcribing || !readable)
            .child(div().text_xs().text_color(theme::muted()).child(style_label(style)))
            .child(chrome::small_icon(IconName::ChevronDown, 12., theme::faint()))
            .dropdown_menu(move |menu, _, cx| {
                let _ = entity.update(cx, |this, cx| this.load_dictation_style(cx));
                STYLES.into_iter().fold(menu, |menu, next| {
                    let entity = entity.clone();
                    menu.item(PopupMenuItem::element(move |_, _| div().flex().flex_col().gap_1().max_w(px(320.))
                        .child(style_label(next)).child(div().text_xs().text_color(theme::muted()).whitespace_normal()
                            .child(tr(&format!("voice_style_{next}_hint")))))
                    .checked(next == style).on_click(move |_, _, cx| {
                        let _ = entity.update(cx, |this, cx| this.set_dictation_style(next, cx));
                    }))
                })
            }).into_any_element());
        let versions = owner && self.dictation.file_name.is_none() && self.dictation.result.is_some()
            && (transcribing || self.dictation.text_in_field(&self.composer.read(cx).value()));
        let has_audio = !self.dictation.audio.lock().unwrap().is_empty();
        let again = owner && self.dictation.result.is_none() && has_audio;
        let file_audio = owner && self.dictation.file_name.is_some() && has_audio;
        let controls = (versions || again || file_audio).then(|| div().flex().flex_wrap().items_center().gap_2()
            .when(versions, |el| {
                let applied = self.dictation.result.as_ref().and_then(|v| v.get("estilo_aplicado")).and_then(Value::as_str).unwrap_or("cru");
                el.child(div().text_xs().text_color(theme::muted()).child(tr("dictation_versions")))
                    .children(["cru", "limpar", "prosa", "briefing"].into_iter().map(|version| {
                        Button::new(SharedString::from(format!("dictation-version-{version}"))).ghost().small()
                            .label(style_label(version)).selected(version == applied).disabled(recording || transcribing)
                            .on_click(cx.listener(move |this, _, window, cx| this.revise_dictation(Some(version), window, cx)))
                    }))
            })
            .when(again, |el| el.child(
                Button::new("dictation-retranscribe").ghost().small().label(tr("dictation_again"))
                    .disabled(recording || transcribing)
                    .on_click(cx.listener(|this, _, window, cx| this.revise_dictation(None, window, cx)))))
            // A gravação que virou o texto, para ouvir de novo antes de enviar.
            .when(!recording && has_audio, |el| el.child(self.audio_controls("dictation", |this, cx| {
                let audio = this.dictation.audio.lock().unwrap().clone();
                let filename = this.dictation.file_name.clone().unwrap_or_else(|| "ditado.wav".into());
                this.toggle_audio("dictation".into(), &filename, async move { Ok(audio) }, cx);
            }, cx))));
        let status = (recording || transcribing).then(|| {
            let label = tr(if recording { "dictation_active" } else if self.dictation.cleaning { "dictation_cleaning" } else { "dictation_working" });
            let seconds = self.dictation.started.map(|start| start.elapsed().as_secs()).unwrap_or(0);
            div().flex().items_center().gap_2().text_sm().text_color(theme::muted())
                .child(div().id("dictation-status").role(Role::Status).aria_label(label.clone()).child(label))
                .when(recording, |el| el
                    .child(div().font_family(theme::MONO).child(format!("{}:{:02}", seconds / 60, seconds % 60)))
                    // Onda: cresce da esquerda até encher a faixa; cheia, a mais nova fica na ponta direita e as velhas
                    // saem pela esquerda (a de dentro encolhe até a largura da de fora e alinha as barras à direita).
                    .child(div().id("dictation-level").role(Role::Meter).aria_label(tr("dictation_level"))
                        .aria_min_numeric_value(0.).aria_max_numeric_value(100.).aria_numeric_value((self.dictation.level * 100.) as f64)
                        .flex_1().min_w_0().h(px(64.)).flex().items_center().overflow_hidden()
                        .child(div().min_w_0().h_full().flex().items_center().justify_end().gap(px(2.)).overflow_hidden()
                            .children(self.dictation.bars.iter().map(|level| div().flex_shrink_0().w(px(3.)).rounded(px(3.))
                                .h(px(8. + level.clamp(0., 1.) * 56.)).bg(theme::accent()))))))
                .when(!recording, |el| el.child(div().flex_1()))
                .child(Button::new("dictation-cancel").ghost().small().label(tr("dictation_cancel"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.cancel_dictation();
                        this.composer.update(cx, |input, cx| input.focus(window, cx));
                        cx.notify();
                    })))
                .into_any_element()
        });
        let countdown = self.dictation.countdown.map(|deadline| {
            let seconds = ((deadline.saturating_duration_since(Instant::now()).as_millis() + 999) / 1000).clamp(1, 3);
            let label = tr("dictation_countdown").replace("{seconds}", &seconds.to_string());
            let owner = cx.entity().downgrade();
            div().flex().items_center().gap_2()
                .child(div().id("dictation-countdown").role(Role::Status).aria_label(label.clone())
                    .text_sm().text_color(theme::accent_text()).child(label))
                .child(Button::new("dictation-countdown-cancel").ghost().small().label(tr("dictation_countdown_cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_dictation_countdown(cx))))
                .child(canvas(|_, _, _| (), move |_, _, window, _| {
                    window.on_mouse_event::<MouseDownEvent>(move |_, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            let _ = owner.update(cx, |this, cx| this.cancel_dictation_countdown(cx));
                        }
                    });
                }).w_0().h_0())
                .into_any_element()
        });
        let error = self.dictation.error.clone().map(|error| div().id("dictation-error").role(Role::Alert)
            .text_sm().text_color(theme::danger()).child(error));
        let busy = controls.is_some() || status.is_some() || countdown.is_some() || error.is_some();
        let strip = (readable && busy).then(|| div().flex().flex_col().gap_2().children(controls).children(status).children(countdown)
            .children(error).into_any_element());
        (mic, pill.filter(|_| readable), strip)
    }

    fn dictation_failure(error: &Failure) -> String {
        match error.status {
            Some(503) => tr("dictation_unconfigured"),
            Some(401 | 403 | 429) => Self::failure(error),
            Some(_) => error.detail.clone(),
            None if error.uncertain => tr("connection_failed"),
            None => tr(&error.detail),
        }
    }
}

fn style_label(style: &str) -> String {
    tr(match style { "limpar" => "voice_style_limpar", "briefing" => "voice_style_briefing", "cru" => "dictation_style_raw", _ => "voice_style_prosa" })
}

fn dictation_insert(value: &str, range: std::ops::Range<usize>, text: &str) -> String {
    let leading = value[..range.start].chars().next_back().is_some_and(|c| !c.is_whitespace());
    let trailing = value[range.end..].chars().next().is_some_and(|c| !c.is_whitespace());
    format!("{}{text}{}", if leading { " " } else { "" }, if trailing { " " } else { "" })
}

#[cfg(test)]
mod tests {
    use super::{dictation_insert, wav, wav_pcm, Dictation, Downmix, Recorder, Vad};
    use std::time::{Duration, Instant};
    #[test]
    fn downmix_turns_48k_stereo_into_16k_mono() {
        let mut mix = Downmix::new(2, 48_000);
        let mut out = Vec::new();
        let frames: Vec<f32> = (0..480).flat_map(|_| [0.5, -0.5]).chain((0..480).flat_map(|_| [1., 1.])).collect();
        mix.push(&frames, |v: f32| v, &mut out);
        let samples: Vec<i16> = out.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(samples.len(), 320, "10 ms a 48 kHz viram 160 amostras a 16 kHz");
        assert_eq!(samples[0], 0, "canais opostos se anulam");
        assert_eq!(samples[319], 32767);
    }
    #[test]
    fn hands_free_waits_for_speech_and_two_seconds_of_silence() {
        let base = Instant::now();
        let mut vad = Vad::default();
        for tick in 0..40 { assert!(!vad.step(0., base + Duration::from_millis(tick * 55))); }
        for tick in 40..70 { assert!(!vad.step(0.3, base + Duration::from_millis(tick * 55))); }
        for tick in 70..107 { assert!(!vad.step(0., base + Duration::from_millis(tick * 55))); }
        assert!(vad.step(0., base + Duration::from_millis(107 * 55)));
    }
    #[test]
    fn versions_preserve_surroundings_and_cancel_releases_audio_without_changing_style() {
        let mut state = Dictation::default();
        assert_eq!(state.style(1), None);
        state.style = Some((1, "limpar"));
        assert_eq!(state.style(2), None);
        state.owner = Some((1, "m".into(), "s".into()));
        let mut draft = "antes ação depois".to_owned();
        state.inserted = Some((draft.clone(), 6..12));
        assert!(!state.text_in_field(""));
        assert!(state.text_in_field(&draft));
        assert!(state.text_in_field("antes ação depois com acréscimo"));
        assert!(!state.text_in_field("antes edição depois"));
        assert!(state.draft_matches(&draft));
        assert!(!state.draft_matches("antes edição depois"));
        let range = state.inserted.as_ref().unwrap().1.clone();
        let replacement = dictation_insert(&draft, range.clone(), "reorganização");
        draft.replace_range(range, &replacement);
        assert_eq!(draft, "antes reorganização depois");
        state.versions.insert("cru".into(), serde_json::json!({"text": "ação"}));
        *state.audio.lock().unwrap() = vec![1, 2];
        let in_flight_audio = state.audio.clone();
        let seq = state.seq;
        state.cancel();
        in_flight_audio.lock().unwrap().push(3);
        assert!(state.audio.lock().unwrap().is_empty());
        assert!(state.versions.is_empty() && state.inserted.is_none() && state.owner.is_none());
        assert_ne!(state.seq, seq);
        assert_eq!(state.style(1), Some("limpar"));
    }
    #[test]
    fn attached_audio_keeps_original_filename_and_bytes_until_cancel() {
        let mut state = Dictation::default();
        state.owner = Some((1, "machine".into(), "session".into()));
        state.file_name = Some("ação gravada.M4A".into());
        let bytes = b"\0\0\0\x18ftypM4A ".to_vec();
        *state.audio.lock().unwrap() = bytes.clone();
        state.error = Some("failed".into());
        assert_eq!(state.file_name.as_deref(), Some("ação gravada.M4A"));
        assert_eq!(*state.audio.lock().unwrap(), bytes);
        assert!(!state.auto_send && state.result.is_none() && state.versions.is_empty());
        let old_seq = state.seq;
        let old_audio = state.audio.clone();
        state.cancel();
        assert!(state.file_name.is_none() && state.owner.is_none() && state.error.is_none());
        assert!(state.audio.lock().unwrap().is_empty());
        assert_eq!(*old_audio.lock().unwrap(), bytes);
        assert_ne!(state.seq, old_seq);
    }
    #[test]
    fn audio_file_generation_changes_even_when_returning_without_active_dictation() {
        let mut state = Dictation::default();
        let original = Some((1, "machine".into(), "session-a".into()));
        let generation = state.observe_file_owner(original.clone());
        assert_eq!(state.observe_file_owner(original.clone()), generation);
        state.observe_file_owner(Some((1, "machine".into(), "session-b".into())));
        assert_ne!(state.observe_file_owner(original.clone()), generation);
        assert!(state.owner.is_none() && state.recorder.is_none() && state.request.is_none());
        let generation = state.observe_file_owner(Some((1, "machine-a".into(), String::new())));
        state.observe_file_owner(Some((1, "machine-b".into(), String::new())));
        assert_ne!(state.observe_file_owner(Some((1, "machine-a".into(), String::new()))), generation);
    }
    #[test]
    fn dictation_preserves_draft_around_cursor_or_selection_and_cancels_old_result() {
        for (value, range, expected) in [
            ("depois", 0..0, "fala depois"), ("antes", 5..5, "antes fala"),
            ("antes depois", 6..6, "antes fala depois"), ("trocar isto", 0..6, "fala isto"),
            ("ação fim", 0..6, "fala fim"), ("tudo", 0..4, "fala"),
            ("", 0..0, "fala"), ("antes\n", 6..6, "antes\nfala"),
        ] {
            let mut result = value.to_owned();
            result.replace_range(range.clone(), &dictation_insert(value, range, "fala"));
            assert_eq!(result, expected);
        }
        let mut state = Dictation::default();
        state.owner = Some((1, "m".into(), "s".into()));
        let old = state.seq;
        state.cancel();
        assert_ne!(state.seq, old);
        assert!(state.owner.is_none());
        let mut pcm = vec![0; 3203];
        pcm[3201] = 128;
        let mut recorder = Recorder { stream: None, failed: Default::default(), pcm: std::sync::Arc::new(std::sync::Mutex::new(pcm)),
            playback: None, sampled: 0, last_signal: (0., 0.), last_pcm_at: None };
        assert_eq!(recorder.signal().0, 1.);
        recorder.last_pcm_at = Some(Instant::now() - Duration::from_millis(200));
        assert_eq!(recorder.signal(), (0., 0.));
    }
    #[test]
    fn wav_roundtrip_rejects_truncation_and_wrong_format() {
        let pcm = [0, 0, 0xff, 0x7f, 0, 0x80];
        let mut bytes = wav(&pcm);
        assert_eq!(wav_pcm(&bytes), Some(pcm.as_slice()));
        assert!(wav_pcm(&bytes[..bytes.len() - 1]).is_none());
        bytes[22] = 2;
        assert!(wav_pcm(&bytes).is_none());
    }
}
