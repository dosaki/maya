//! The parts of Maya's ear that are plain logic, ported from the Swift ear
//! (`ear/vad.swift`) so both behave alike: voice activity detection, the
//! filler filter, resampling to 16 kHz, and the microphone choice.

pub mod resample;
pub mod vad;

/// Inputs that are never a microphone: loopbacks, virtual cables and
/// remote-desktop audio. Checked before honouring a preferred name.
const VIRTUAL: &[&str] = &["blackhole", "remote sound", "soundflower", "loopback", "stereo mix", "vb-audio", "cable output", "voicemeeter", "remote audio", "virtual audio"];

pub fn is_virtual(name: &str) -> bool {
    let n = name.to_lowercase();
    VIRTUAL.iter().any(|v| n.contains(v))
}

/// The preferred device when present and real, else the system's default
/// input when real, else the first real input.
pub fn pick_device(names: &[String], preferred: Option<&str>, default: Option<&str>) -> Option<String> {
    let ok: Vec<&String> = names.iter().filter(|n| !is_virtual(n)).collect();
    if let Some(p) = preferred.and_then(|p| ok.iter().find(|n| n.as_str() == p)) {
        return Some(p.to_string());
    }
    if let Some(d) = default.and_then(|d| ok.iter().find(|n| n.as_str() == d)) {
        return Some(d.to_string());
    }
    ok.first().map(|n| n.to_string())
}

/// RMS of `samples`, 0 for none.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// The level meter's value: typical speech RMS (0.05-0.15) scaled by 8
/// into 0-1, as the Swift ear reports it.
pub fn level(rms: f32) -> f64 {
    (rms as f64 * 8.0).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn virtual_inputs_are_never_picked() {
        let n = names(&["Stereo Mix (Realtek(R) Audio)", "CABLE Output (VB-Audio Virtual Cable)", "Microphone Array (Intel Smart Sound)"]);
        assert_eq!(pick_device(&n, Some("Stereo Mix (Realtek(R) Audio)"), None).as_deref(), Some("Microphone Array (Intel Smart Sound)"));
        assert_eq!(pick_device(&names(&["Stereo Mix (Realtek(R) Audio)"]), None, None), None);
    }

    #[test]
    fn preferred_then_default_then_first() {
        let n = names(&["Headset (USB)", "Microphone (Realtek(R) Audio)"]);
        assert_eq!(pick_device(&n, Some("Headset (USB)"), Some("Microphone (Realtek(R) Audio)")).as_deref(), Some("Headset (USB)"));
        assert_eq!(pick_device(&n, Some("Gone"), Some("Microphone (Realtek(R) Audio)")).as_deref(), Some("Microphone (Realtek(R) Audio)"));
        assert_eq!(pick_device(&n, None, None).as_deref(), Some("Headset (USB)"));
    }

    #[test]
    fn level_scales_and_clamps() {
        assert_eq!(rms(&[]), 0.0);
        assert!((rms(&[0.1, -0.1]) - 0.1).abs() < 1e-6);
        assert!((level(0.1) - 0.8).abs() < 1e-6);
        assert_eq!(level(0.5), 1.0);
    }
}
