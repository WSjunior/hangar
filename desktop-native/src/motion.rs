// Kit de movimento: o catálogo do Zeron (tempos, curvas e o jeito de cada entrada) sobre a GPUI do gpui-kit.
//
// Adaptado de `crates/ui/src/motion.rs` do Zeron (github.com/zeronsh, revisão c338a6e), sob esta licença:
//
//   MIT License
//
//   Copyright (c) 2026 Wing
//
//   Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated
//   documentation files (the "Software"), to deal in the Software without restriction, including without limitation
//   the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and
//   to permit persons to whom the Software is furnished to do so, subject to the following conditions:
//
//   The above copyright notice and this permission notice shall be included in all copies or substantial portions of
//   the Software.
//
//   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
//   THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//   AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF
//   CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
//   IN THE SOFTWARE.
//
// O `cubic-bezier` exato é o do gpui-base, o mesmo algoritmo do Zeron. A GPUI não escala `div`: `menu-in` e `dialog-in`
// fazem a escala do Zeron com opacidade e deslocamento. O deslocamento é o `top` relativo, que não empurra os vizinhos.
//
// Quem pede o quadro seguinte importa. Uma animação na raiz que avisasse a view da raiz acordaria todas as áreas, que a
// observam (`panes.rs`); aqui a raiz pede o quadro pelo `ticker`, uma view vazia dentro dela, e só ela redesenha. Dentro
// de uma área, o quadro é só dessa área.
use gpui_kit::{base::animation::cubic_bezier, *};
use std::time::{Duration, Instant};

/// Uma entrada do catálogo: duração e curva `cubic-bezier`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec { ms: u64, delay: u64, curve: [f32; 4] }

const EASE: [f32; 4] = [0.25, 0.1, 0.25, 1.];
const EASE_OUT: [f32; 4] = [0., 0., 0.58, 1.];
const EASE_OUT_EXPO: [f32; 4] = [0.16, 1., 0.3, 1.];
const EASE_OUT_QUINT: [f32; 4] = [0.22, 1., 0.36, 1.];
/// `--ease-out` do web (`cubic-bezier(0.23, 1, 0.32, 1)`), a curva da marca e da linha "trabalhando".
const EASE_OUT_WEB: [f32; 4] = [0.23, 1., 0.32, 1.];

/// `fade-in`: entradas, 0,5 s, opacidade e 4 px subindo.
pub const FADE_IN: Spec = Spec::new(500, EASE_OUT_EXPO);
/// `fade-quick`: troca de tela ou aba, 0,15 s só de opacidade.
pub const FADE_QUICK: Spec = Spec::new(150, EASE);
/// `menu-in`: popover abrindo, 0,14 s.
pub const MENU_IN: Spec = Spec::new(140, EASE);
/// Popover fechando: 0,1 s, mais curto que a entrada.
pub const MENU_OUT: Spec = Spec::new(100, EASE);
/// `dialog-in`: 0,18 s.
pub const DIALOG_IN: Spec = Spec::new(180, EASE);
/// Painéis abrindo e fechando: 200 ms `ease-out`.
pub const RESIZE: Spec = Spec::new(200, EASE_OUT);
/// `splash-out`: a tela que sai, 0,5 s depois de 0,15 s parada, opacidade e 6 px subindo.
pub const SPLASH_OUT: Spec = Spec::new(500, EASE).after(150);
/// A linha "trabalhando" e a marca dela entrando (tempo e curva do web).
pub const WORKING: Spec = Spec::new(200, EASE_OUT_WEB);
/// A barra de sessões recolhendo ao trilho e voltando: o `transition: width 160ms var(--ease-out)` do web.
pub const NAV_FOLD: Spec = Spec::new(160, EASE_OUT_WEB);
/// Ferramenta nova entrando no grupo da Árvore: altura, opacidade e 4 px subindo.
pub const TOOL_REVEAL: Spec = Spec::new(360, EASE_OUT_EXPO);
/// Intervalo entre ferramentas que chegam juntas, para entrarem uma de cada vez.
pub const TOOL_STAGGER: Duration = Duration::from_millis(65);
/// Grupo da Árvore abrindo e fechando.
pub const TOOL_FOLD: Spec = Spec::new(140, EASE_OUT);
/// O traço da árvore descendo até a ponta da ferramenta nova: rápido no começo, assentando na ponta.
pub const TOOL_CONNECTOR: Spec = Spec::new(480, EASE_OUT_QUINT);
/// Uma passada do brilho no título do grupo que ainda roda.
pub const TOOL_SHIMMER: Duration = Duration::from_millis(3400);
/// `zeron-pulse`: período dos esqueletos de carregando.
pub const PULSE: Duration = Duration::from_millis(2400);

impl Spec {
    const fn new(ms: u64, curve: [f32; 4]) -> Self { Self { ms, delay: 0, curve } }

    pub const fn after(mut self, delay: u64) -> Self { self.delay = delay; self }

    pub const fn duration(self) -> Duration { Duration::from_millis(self.ms) }

    /// Espera mais duração: quando a linha do tempo inteira acaba.
    pub fn total(self) -> Duration { Duration::from_millis(self.delay + self.ms).mul_f32(scale()) }

    /// Progresso com a curva para `raw` de 0 a 1 do tempo; fora disso, preso às pontas.
    pub fn ease(self, raw: f32) -> f32 {
        let [x1, y1, x2, y2] = self.curve;
        cubic_bezier(x1, y1, x2, y2)(raw)
    }

    /// Quanto do tempo passou desde `start`, de 0 a 1, sem curva; 0 durante a espera.
    pub fn raw(self, start: Instant) -> f32 {
        let (delay, span) = (self.delay as f32 / 1000. * scale(), self.duration().as_secs_f32() * scale());
        ((start.elapsed().as_secs_f32() - delay) / span).clamp(0., 1.)
    }
}

/// Só para medir e provar: `HANGAR_NATIVE_MOTION_SCALE=8` estica 8 vezes cada linha do tempo do catálogo, para uma
/// sequência de capturas pegar os quadros do meio. Sem a variável, 1.
fn scale() -> f32 {
    static SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SCALE.get_or_init(|| std::env::var("HANGAR_NATIVE_MOTION_SCALE").ok().and_then(|v| v.parse::<f32>().ok())
        .filter(|s| s.is_finite()).map_or(1., |s| s.clamp(0.1, 50.)))
}

/// A curva `--ease-out` do web para quem monta os próprios trechos (a marca "trabalhando").
pub fn ease_out(x: f32) -> f32 { WORKING.ease(x) }

/// Onda do pulso: 0 no começo do ciclo, 1 no meio, 0 no fim.
pub fn pulse_wave(phase: f32) -> f32 { 0.5 - 0.5 * (phase * std::f32::consts::TAU).cos() }

// Os desenhos de cada entrada, para `t` já com a curva (0 = começo, 1 = no lugar).

pub fn fade_in<E: Styled>(el: E, t: f32) -> E { el.relative().opacity(t).top(px(4. * (1. - t))) }

/// A tela sem sessão chegando: desce 10 px até o lugar, como um objeto só.
pub fn settle_down<E: Styled>(el: E, t: f32) -> E { el.relative().opacity(t).top(px(-10. * (1. - t))) }

pub fn fade_quick<E: Styled>(el: E, t: f32) -> E { el.opacity(t) }

pub fn menu_in<E: Styled>(el: E, t: f32) -> E { el.relative().opacity(0.3 + 0.7 * t).top(px(-2. * (1. - t))) }

/// Saída do popover; `t` vai de 0 (inteiro) a 1 (sumido).
pub fn menu_out<E: Styled>(el: E, t: f32) -> E { el.relative().opacity(1. - t).top(px(-2. * t)) }

/// Um traço da árvore chegando, de `progress` (0 a 1, com curva): quanto do tronco que desce até a linha e quanto do
/// ramo até ela estão desenhados. Com uma linha antes, o tronco dela desce primeiro (`connector_continuation`) e só
/// depois o desta; o ramo começa um pouco antes do fim do tronco, sem quadro parado na curva.
pub fn connector_parts(progress: f32, has_predecessor: bool) -> (f32, f32) {
    let progress = progress.clamp(0., 1.);
    let (start, end, branch) = if has_predecessor { (0.45, 0.72, 0.68) } else { (0., 0.62, 0.58) };
    (((progress - start) / (end - start)).clamp(0., 1.), ((progress - branch) / (1. - branch)).clamp(0., 1.))
}

/// O tronco que sai da linha de cima até a próxima: pertence à de cima, mas anda no tempo da que chega.
pub fn connector_continuation(next_progress: Option<f32>) -> f32 {
    next_progress.map_or(1., |progress| (progress / 0.45).clamp(0., 1.))
}

/// Quanto brilha o ponto `x` (0 a 1 da largura do título) na fase `phase` da passada: faixa de 0,36 título de meia
/// largura, cópias a três títulos uma da outra, como no Zeron.
pub fn shimmer_amount(x: f32, phase: f32) -> f32 {
    let center = -2.5 + phase.clamp(0., 1.) * 6.;
    (-2..=2).map(|copy| center + copy as f32 * 3.).map(|c| (1. - (x - c).abs() / 0.36).clamp(0., 1.)).fold(0., f32::max)
}

/// Progresso com curva da entrada do elemento `key`, contado do primeiro desenho dele; pede quadros até terminar. O
/// relógio vive enquanto o elemento é desenhado em quadros seguidos: sumiu e voltou, entra de novo. Com movimento
/// reduzido, já chega inteiro.
pub fn enter(key: impl Into<ElementId>, spec: Spec, window: &mut Window, cx: &App) -> f32 {
    if cx.reduce_motion() { return 1.; }
    let start = window.with_global_id(key.into(), |id, window| window.with_element_state(id, |start: Option<Instant>, _| {
        let start = start.unwrap_or_else(Instant::now);
        (start, start)
    }));
    if start.elapsed() < spec.total() { request_frame(window, cx); }
    spec.ease(spec.raw(start))
}

/// A view vazia que a raiz monta para pedir quadros só para si.
struct Tick;

impl Render for Tick {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { div() }
}

struct Ticker { view: Entity<Tick>, root: EntityId }

impl Global for Ticker {}

/// Monta o relógio da raiz: chamar uma vez no desenho da view de cima, em todo quadro.
pub fn ticker(window: &mut Window, cx: &mut App) -> AnyElement {
    let root = window.current_view();
    if !cx.has_global::<Ticker>() {
        let view = cx.new(|_| Tick);
        cx.set_global(Ticker { view, root });
    }
    let ticker = cx.global_mut::<Ticker>();
    ticker.root = root;
    AnyView::from(ticker.view.clone()).cached(StyleRefinement::default().absolute().size_0()).into_any_element()
}

/// Redesenha só a raiz, fora do desenho: as áreas guardadas são reusadas. `false` sem o relógio montado.
pub fn tick_root(cx: &mut App) -> bool {
    let Some(tick) = cx.try_global::<Ticker>().map(|ticker| ticker.view.entity_id()) else { return false };
    cx.notify(tick);
    true
}

/// Pede o próximo quadro para quem está desenhando: na raiz, pelo relógio dela; numa área ou view própria, só ela.
pub fn request_frame(window: &Window, cx: &App) {
    match cx.try_global::<Ticker>().filter(|ticker| ticker.root == window.current_view()) {
        Some(ticker) => {
            let tick = ticker.view.entity_id();
            window.on_next_frame(move |_, cx| cx.notify(tick));
        }
        None => window.request_animation_frame(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // O glob pode trazer o `test` da gpui, que colide com o atributo padrão; o nome explícito vence o glob.
    use core::prelude::v1::test;

    #[test]
    fn catalog_matches_zeron() {
        assert_eq!((FADE_IN.ms, FADE_QUICK.ms, MENU_IN.ms, MENU_OUT.ms, DIALOG_IN.ms, RESIZE.ms, SPLASH_OUT.ms, SPLASH_OUT.delay),
            (500, 150, 140, 100, 180, 200, 500, 150));
        for spec in [FADE_IN, FADE_QUICK, MENU_IN, DIALOG_IN, RESIZE, SPLASH_OUT, WORKING] {
            assert_eq!((spec.ease(0.), spec.ease(1.), spec.ease(2.)), (0., 1., 1.));
            let mut last = 0.;
            for i in 0..=100 {
                let y = spec.ease(i as f32 / 100.);
                assert!((0. ..=1.).contains(&y) && y >= last - 1e-4);
                last = y;
            }
        }
        // Valores de referência do Zeron (bisseção de 80 passos): expo em 0,5 e `ease` em 0,25.
        assert!((FADE_IN.ease(0.5) - 0.971779).abs() < 1e-3 && (FADE_QUICK.ease(0.25) - 0.408511).abs() < 1e-3);
    }

    #[test]
    fn connector_draws_the_previous_trunk_first_and_ends_whole() {
        assert_eq!((connector_parts(0., false), connector_parts(1., false), connector_parts(1., true)), ((0., 0.), (1., 1.), (1., 1.)));
        // Com linha antes, nada desta nasce enquanto o tronco de cima desce.
        assert_eq!(connector_parts(0.4, true), (0., 0.));
        assert_eq!((connector_continuation(Some(0.)), connector_continuation(Some(0.45)), connector_continuation(None)), (0., 1., 1.));
        assert!(shimmer_amount(0.5, 0.5) > 0.99 && shimmer_amount(0.5, 0.25) < 0.01);
    }

    #[test]
    fn pulse_ends() {
        assert!((pulse_wave(0.) - 0.).abs() < 1e-6 && (pulse_wave(0.5) - 1.).abs() < 1e-6);
    }
}
