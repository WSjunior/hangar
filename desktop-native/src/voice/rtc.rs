//! Chamada WebRTC direto com a OpenAI: o app-server só troca o SDP; o áudio não passa por ele.
use crate::voice::audio::{Audio, AudioError, FRAME};
use std::{net::{IpAddr, SocketAddr, UdpSocket}, sync::{Arc, Once, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcConfig,
    change::{SdpAnswer, SdpPendingOffer}, format::Codec, media::{Direction, Frequency, MediaKind, MediaTime, Mid}, net::{Protocol, Receive}};

// UDP bloqueado não dá erro: só nunca conecta. Sem prazo a tela fica em "conectando" para sempre.
const CONNECT_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy)]
pub enum RtcError { Network, Answer, Media, Microphone, Speaker }

pub enum RtcEvent { Connected, Closed, Levels(f32, f32), Failed(RtcError) }

pub struct Offer { pub sdp: String, rtc: Rtc, socket: UdpSocket, local: SocketAddr, mid: Mid, pending: SdpPendingOffer }

static CRYPTO: Once = Once::new();

/// O IP que sai para a internet; o socket escuta em 0.0.0.0, mas o candidato precisa do endereço real.
fn route_ip() -> Option<IpAddr> {
    let probe = UdpSocket::bind("0.0.0.0:0").ok()?;
    probe.connect("8.8.8.8:80").ok()?;
    Some(probe.local_addr().ok()?.ip())
}

pub fn offer() -> Result<Offer, RtcError> {
    CRYPTO.call_once(|| str0m::crypto::from_feature_flags().install_process_default());
    let mut rtc = RtcConfig::new().build(Instant::now());
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|_| RtcError::Network)?;
    let ip = route_ip().ok_or(RtcError::Network)?;
    let local = SocketAddr::new(ip, socket.local_addr().map_err(|_| RtcError::Network)?.port());
    rtc.add_local_candidate(Candidate::host(local, "udp").map_err(|_| RtcError::Network)?).ok_or(RtcError::Network)?;
    let mut api = rtc.sdp_api();
    let mid = api.add_media(MediaKind::Audio, Direction::SendRecv, None, None, None);
    // O nome é o que o Realtime espera; sem o canal a OpenAI não abre a sessão.
    api.add_channel("oai-events".into());
    let (offer, pending) = api.apply().ok_or(RtcError::Media)?;
    Ok(Offer { sdp: offer.to_sdp_string(), rtc, socket, local, mid, pending })
}

// O canal de eventos precisa ser ilimitado: com send_blocking num canal cheio o laço de mídia travaria.
pub fn run(offer: Offer, answer: String, muted: Arc<AtomicBool>, events: async_channel::Sender<RtcEvent>, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let outcome = drive(offer, answer, muted, &events, &stop);
        let _ = events.send_blocking(match outcome { Ok(()) => RtcEvent::Closed, Err(error) => RtcEvent::Failed(error) });
    })
}

fn audio_error(error: AudioError) -> RtcError {
    match error { AudioError::Microphone => RtcError::Microphone, AudioError::Speaker => RtcError::Speaker }
}

fn drive(offer: Offer, answer: String, muted: Arc<AtomicBool>, events: &async_channel::Sender<RtcEvent>, stop: &AtomicBool) -> Result<(), RtcError> {
    let Offer { mut rtc, socket, local, mid, pending, .. } = offer;
    let answer = SdpAnswer::from_sdp_string(&answer).map_err(|_| RtcError::Answer)?;
    rtc.sdp_api().accept_answer(pending, answer).map_err(|_| RtcError::Answer)?;
    // Abrir o som aqui: o fluxo do cpal fica na thread que o usa, e a tela não espera o WASAPI.
    let mut audio = Audio::start(muted).map_err(audio_error)?;
    let mut encoder = opus_rs::OpusEncoder::new(48_000, 1, opus_rs::Application::Voip).map_err(|_| RtcError::Media)?;
    let mut decoder = opus_rs::OpusDecoder::new(48_000, 1).map_err(|_| RtcError::Media)?;
    let (mut connected, mut timestamp, mut buffer) = (false, 0u64, vec![0u8; 2000]);
    let mut write_errors = 0u32;
    let (mut decoded, mut packet) = (vec![0f32; FRAME * 2], vec![0u8; 1500]);
    let (started, mut last_levels) = (Instant::now(), Instant::now());
    let result = loop {
        if stop.load(Ordering::Relaxed) { break Ok(()); }
        // Ninguém ouve mais: sem isto o microfone ficaria aberto.
        if events.is_closed() { break Err(RtcError::Network); }
        if !connected && started.elapsed() > CONNECT_DEADLINE { break Err(RtcError::Network); }
        if let Some(error) = audio.failed() { break Err(audio_error(error)); }
        let timeout = match rtc.poll_output() {
            Err(_) => break Err(RtcError::Network),
            Ok(Output::Transmit(t)) => { let _ = socket.send_to(&t.contents, t.destination); continue; }
            Ok(Output::Event(event)) => {
                match event {
                    Event::Connected => { connected = true; audio.reset(); let _ = events.send_blocking(RtcEvent::Connected); }
                    Event::IceConnectionStateChange(IceConnectionState::Disconnected) => break Err(RtcError::Network),
                    Event::MediaData(media) => {
                        if let Ok(n) = decoder.decode(&media.data, FRAME, &mut decoded) { audio.play(&decoded[..n]); }
                    }
                    _ => {}
                }
                continue;
            }
            Ok(Output::Timeout(t)) => t,
        };
        // Mídia escrita antes do Connected é descartada pelo str0m.
        if connected {
            let mut return_media_failure = false;
            while let Some(frame) = audio.next_frame() {
                let Ok(len) = encoder.encode(&frame, FRAME, &mut packet) else { continue };
                let Some(writer) = rtc.writer(mid) else { break };
                let Some(pt) = writer.payload_params().find(|p| p.spec().codec == Codec::Opus).map(|p| p.pt()) else { break };
                // Falha persistente de escrita deixaria a chamada conectada com o microfone mudo para a OpenAI.
                match writer.write(pt, Instant::now(), MediaTime::new(timestamp, Frequency::FORTY_EIGHT_KHZ), packet[..len].to_vec()) {
                    Ok(_) => write_errors = 0,
                    Err(_) => { write_errors += 1; if write_errors >= 50 { return_media_failure = true; break; } }
                }
                timestamp += FRAME as u64;
            }
            if return_media_failure { break Err(RtcError::Media); }
            if last_levels.elapsed() >= Duration::from_millis(60) {
                let (input, output) = audio.levels();
                let _ = events.try_send(RtcEvent::Levels(input, output));
                last_levels = Instant::now();
            }
        }
        // Acorda no mínimo a cada 10 ms para drenar o microfone mesmo sem pacote chegando.
        let wait = timeout.saturating_duration_since(Instant::now()).min(Duration::from_millis(10));
        if wait.is_zero() {
            if rtc.handle_input(Input::Timeout(Instant::now())).is_err() { break Err(RtcError::Network); }
            continue;
        }
        let _ = socket.set_read_timeout(Some(wait));
        match socket.recv_from(&mut buffer) {
            Ok((n, source)) => {
                let Ok(contents) = buffer[..n].try_into() else { continue };
                if rtc.handle_input(Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source, destination: local, contents })).is_err() { break Err(RtcError::Network); }
            }
            Err(_) => { if rtc.handle_input(Input::Timeout(Instant::now())).is_err() { break Err(RtcError::Network); } }
        }
    };
    rtc.disconnect();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn offer_has_opus_audio_and_events_channel() {
        // Máquina sem rota de rede não monta a oferta.
        let Ok(offer) = offer() else { return };
        assert!(offer.sdp.starts_with("v=0"));
        assert!(offer.sdp.contains("m=audio"));
        assert!(offer.sdp.to_lowercase().contains("opus/48000"));
        assert!(offer.sdp.contains("webrtc-datachannel"));
    }
}
