//! The wheel's feel: a soft click when it opens, a detent tick as the
//! highlight moves to another option, and a firmer press when one is picked.
//!
//! - Sounds (Settings > Wheel > Sounds, off by default): two very short WAVs
//!   made here at start-up (three, with the opening). Windows plays them with PlaySound (shared mode,
//!   no audio session kept open, so games, Discord and ShadowPlay are not
//!   touched); macOS with AudioServices' system sounds. Other systems: silent.
//! - Haptics (macOS only, Settings > Wheel > Trackpad ticks, on by default):
//!   NSHapticFeedbackManager, felt on Force Touch trackpads. A mouse feels
//!   nothing, and nothing is played instead.
//!
//! Ticks closer together than MIN_GAP_MS are dropped (a fast sweep across
//! the ring would otherwise sound like a rattle). A pick always plays.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// The shortest gap between two ticks.
pub const MIN_GAP_MS: u64 = 35;

const RATE: u32 = 44_100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The highlight moved to another option.
    Tick,
    /// An option was picked.
    Pick,
    /// The wheel opened.
    Open,
}

impl Kind {
    pub fn from_word(w: &str) -> Option<Kind> {
        match w {
            "tick" => Some(Kind::Tick),
            "pick" => Some(Kind::Pick),
            "open" => Some(Kind::Open),
            _ => None,
        }
    }
}

static LAST_TICK: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// True when a tick at `now` may play (and records it).
fn tick_allowed(now: u64) -> bool {
    let last = LAST_TICK.load(Ordering::Relaxed);
    if now.saturating_sub(last) < MIN_GAP_MS && last != 0 {
        return false;
    }
    LAST_TICK.store(now, Ordering::Relaxed);
    true
}

/// What the wheel asks for: plays the sound and/or the haptic the settings
/// allow. Returns what was played (for the log and tests).
pub fn wheel(app: &tauri::AppHandle, kind: Kind) -> (bool, bool) {
    let s = crate::settings::get();
    let sound = s.wheel_sounds;
    let haptic = s.wheel_haptics && cfg!(target_os = "macos");
    if !sound && (!haptic || kind == Kind::Open) {
        return (false, false);
    }
    if kind == Kind::Tick && !tick_allowed(now_ms()) {
        return (false, false);
    }
    if sound {
        play(kind);
    }
    // The trackpad only ticks for moving and picking; opening is sound only.
    if haptic && kind != Kind::Open {
        haptic_on_main(app, kind);
    }
    (sound, haptic)
}

/// Settings' "Try it" button: plays the sound whatever the setting says.
pub fn preview(kind: Kind) {
    play(kind);
}

// ---------------------------------------------------------------------------
// The sounds, made once.

/// 16-bit mono WAV from samples in -1..1.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

/// One-pole low-pass (`a` near 1: darker).
fn low_pass(x: &mut [f32], a: f32) {
    let mut y = 0.0;
    for v in x.iter_mut() {
        y = a * y + (1.0 - a) * *v;
        *v = y;
    }
}

/// Repeatable white noise.
fn noise(n: usize, mut seed: u32) -> Vec<f32> {
    (0..n)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 9) as f32 / (1u32 << 23) as f32 * 2.0 - 1.0
        })
        .collect()
}

/// A detent click, like a scroll wheel or a rotary knob: a dark, very short
/// noise snap for the edge, a short 1.1 kHz knock and a little 300 Hz body
/// for weight. Lower and shorter than a beep, so it reads as touch, not tone.
fn click(n: usize, gain: f32, seed: u32) -> Vec<f32> {
    let mut snap = noise(n, seed);
    low_pass(&mut snap, 0.55);
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let attack = (t / 0.00015).min(1.0);
            let edge = snap[i] * (-t / 0.0004).exp();
            let knock = (std::f32::consts::TAU * 1100.0 * t).sin() * (-t / 0.0011).exp();
            let body = (std::f32::consts::TAU * 300.0 * t).sin() * (-t / 0.0028).exp();
            gain * attack * (0.9 * edge + 0.75 * knock + 0.45 * body)
        })
        .collect()
}

/// The tick: one detent click, 12 ms.
fn tick_samples() -> Vec<f32> {
    click((RATE as f32 * 0.012) as usize, 0.11, 0x9E37_79B9)
}

/// The pick: a firmer press (the click, a bit heavier) and a short, soft
/// confirming note under it.
fn pick_samples() -> Vec<f32> {
    let n = (RATE as f32 * 0.06) as usize;
    let press = click(n, 0.115, 0x51ED_270B);
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let second = t - 0.012;
            let note = if second > 0.0 {
                (std::f32::consts::TAU * 880.0 * second).sin() * (-second / 0.012).exp() * (second / 0.002).min(1.0)
            } else {
                0.0
            };
            press[i] + 0.045 * note
        })
        .collect()
}

/// The wheel opening: a soft, low "fwip" (a quick upward glide with a little
/// air), quieter than the tick so it doesn't announce itself.
fn open_samples() -> Vec<f32> {
    let n = (RATE as f32 * 0.075) as usize;
    let mut air = noise(n, 0x2545_F491);
    low_pass(&mut air, 0.9);
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let env = (t / 0.010).min(1.0) * (-(t - 0.010).max(0.0) / 0.018).exp();
            // 420 Hz to 760 Hz over the first 40 ms.
            let f = 420.0 + 340.0 * (t / 0.04).min(1.0);
            phase += std::f32::consts::TAU * f / RATE as f32;
            0.036 * env * (phase.sin() + 0.8 * air[i])
        })
        .collect()
}

pub fn sound(kind: Kind) -> &'static [u8] {
    static TICK: OnceLock<Vec<u8>> = OnceLock::new();
    static PICK: OnceLock<Vec<u8>> = OnceLock::new();
    static OPEN: OnceLock<Vec<u8>> = OnceLock::new();
    match kind {
        Kind::Tick => TICK.get_or_init(|| wav(&tick_samples())),
        Kind::Pick => PICK.get_or_init(|| wav(&pick_samples())),
        Kind::Open => OPEN.get_or_init(|| wav(&open_samples())),
    }
}

/// Makes the sounds (and on macOS loads them) before the wheel first needs
/// them, so the first tick isn't late.
pub fn prepare() {
    let _ = sound(Kind::Tick);
    let _ = sound(Kind::Pick);
    let _ = sound(Kind::Open);
    #[cfg(target_os = "macos")]
    mac::prepare();
}

// ---------------------------------------------------------------------------
// Windows

#[cfg(windows)]
fn play(kind: Kind) {
    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    let bytes = sound(kind);
    // SND_MEMORY: the "name" is the WAV itself. The bytes live for the whole
    // run (OnceLock), so playing on asynchronously is safe. A new sound stops
    // the one before, which is what a quick sweep wants.
    let ok = unsafe { PlaySoundW(PCWSTR(bytes.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT) };
    if !ok.as_bool() {
        log::debug!("wheel sound didn't play");
    }
}

// ---------------------------------------------------------------------------
// macOS

#[cfg(target_os = "macos")]
fn play(kind: Kind) {
    mac::play(kind);
}

#[cfg(target_os = "macos")]
fn haptic_on_main(app: &tauri::AppHandle, kind: Kind) {
    let _ = app.run_on_main_thread(move || mac::haptic(kind));
}

#[cfg(not(target_os = "macos"))]
fn haptic_on_main(_app: &tauri::AppHandle, _kind: Kind) {}

#[cfg(not(any(windows, target_os = "macos")))]
fn play(_kind: Kind) {}

#[cfg(target_os = "macos")]
mod mac {
    use super::Kind;
    use std::ffi::{c_char, c_void};
    use std::sync::OnceLock;

    type CFURLRef = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(alloc: *const c_void, buffer: *const u8, len: isize, is_dir: u8) -> CFURLRef;
        fn CFRelease(cf: *const c_void);
    }
    #[link(name = "AudioToolbox", kind = "framework")]
    extern "C" {
        fn AudioServicesCreateSystemSoundID(url: CFURLRef, out: *mut u32) -> i32;
        fn AudioServicesPlaySystemSound(id: u32);
    }
    #[link(name = "AppKit", kind = "framework")]
    extern "C" {}
    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
    }

    /// System sound ids for the tick, the pick and the opening (0: couldn't load).
    fn ids() -> &'static (u32, u32, u32) {
        static IDS: OnceLock<(u32, u32, u32)> = OnceLock::new();
        IDS.get_or_init(|| (load(Kind::Tick, "tick"), load(Kind::Pick, "pick"), load(Kind::Open, "open")))
    }

    fn load(kind: Kind, name: &str) -> u32 {
        let path = std::env::temp_dir().join(format!("convertino-wheel-{name}.wav"));
        if std::fs::write(&path, super::sound(kind)).is_err() {
            log::warn!("couldn't write the wheel sound to {}", path.display());
            return 0;
        }
        let p = path.to_string_lossy().into_owned();
        let mut id = 0u32;
        unsafe {
            let url = CFURLCreateFromFileSystemRepresentation(std::ptr::null(), p.as_ptr(), p.len() as isize, 0);
            if url.is_null() {
                return 0;
            }
            let err = AudioServicesCreateSystemSoundID(url, &mut id);
            CFRelease(url);
            if err != 0 {
                log::warn!("couldn't load the wheel sound ({err})");
                return 0;
            }
        }
        id
    }

    pub fn prepare() {
        let _ = ids();
    }

    pub fn play(kind: Kind) {
        let (tick, pick, open) = *ids();
        let id = match kind {
            Kind::Tick => tick,
            Kind::Pick => pick,
            Kind::Open => open,
        };
        if id != 0 {
            unsafe { AudioServicesPlaySystemSound(id) };
        }
    }

    /// NSHapticFeedbackManager.defaultPerformer performFeedbackPattern:performanceTime:
    /// Alignment (1) for a tick, Generic (0) for a pick; Now (1). Main thread only.
    pub fn haptic(kind: Kind) {
        unsafe {
            let cls = objc_getClass(c"NSHapticFeedbackManager".as_ptr());
            if cls.is_null() {
                return;
            }
            let send0: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void = std::mem::transmute(objc_msgSend as *const ());
            let performer = send0(cls, sel_registerName(c"defaultPerformer".as_ptr()));
            if performer.is_null() {
                return;
            }
            let send2: unsafe extern "C" fn(*mut c_void, *mut c_void, isize, usize) = std::mem::transmute(objc_msgSend as *const ());
            let pattern: isize = if kind == Kind::Tick { 1 } else { 0 };
            send2(performer, sel_registerName(c"performFeedbackPattern:performanceTime:".as_ptr()), pattern, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_are_short_quiet_wavs() {
        for (kind, max_ms) in [(Kind::Tick, 15), (Kind::Pick, 70), (Kind::Open, 80)] {
            let b = sound(kind);
            assert_eq!(&b[0..4], b"RIFF");
            assert_eq!(&b[8..12], b"WAVE");
            let samples = (b.len() - 44) / 2;
            assert!(samples * 1000 / RATE as usize <= max_ms, "{kind:?} is too long");
            let peak = b[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap();
            // Quiet: well under full scale (about -15 dBFS or lower), but audible.
            assert!(peak < i16::MAX as u16 / 6, "{kind:?} peak {peak}");
            assert!(peak > 1000, "{kind:?} is silent");
            // Starts at silence (no click from a jump).
            let first = i16::from_le_bytes([b[44], b[45]]).unsigned_abs();
            assert!(first < 200);
        }
    }

    #[test]
    fn ticks_too_close_are_dropped() {
        LAST_TICK.store(0, Ordering::Relaxed);
        assert!(tick_allowed(10_000));
        assert!(!tick_allowed(10_000 + MIN_GAP_MS - 1));
        assert!(tick_allowed(10_000 + MIN_GAP_MS));
    }

    /// Writes the three sounds to $CONVERTINO_FEEL_DUMP to listen to them.
    #[test]
    #[ignore]
    fn dump_sounds() {
        let Some(dir) = std::env::var_os("CONVERTINO_FEEL_DUMP") else { return };
        let dir = std::path::PathBuf::from(dir);
        for (kind, name) in [(Kind::Open, "open"), (Kind::Tick, "tick"), (Kind::Pick, "pick")] {
            std::fs::write(dir.join(format!("wheel-{name}.wav")), sound(kind)).unwrap();
        }
    }

    #[test]
    fn words() {
        assert_eq!(Kind::from_word("tick"), Some(Kind::Tick));
        assert_eq!(Kind::from_word("pick"), Some(Kind::Pick));
        assert_eq!(Kind::from_word("open"), Some(Kind::Open));
        assert_eq!(Kind::from_word("boom"), None);
    }
}
