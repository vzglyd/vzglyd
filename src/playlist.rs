use std::path::Path;

use serde::{Deserialize, Serialize};

pub const PLAYLIST_FILENAME: &str = "playlist.json";

/// Top-level structure for `slides/playlist.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Playlist {
    /// Default display settings applied to any entry that does not specify its own.
    #[serde(default)]
    pub defaults: PlaylistDefaults,
    /// Ordered list of slides to display.
    pub slides: Vec<PlaylistEntry>,
}

/// Fallback display settings for entries that do not override them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaylistDefaults {
    /// How long each slide is shown (seconds). Overrides the engine default; overridden by per-entry value.
    pub duration_seconds: Option<u32>,
    /// Transition played when this slide enters the screen.
    pub transition_in: Option<String>,
    /// Transition played when this slide leaves the screen.
    pub transition_out: Option<String>,
}

/// A single entry in the `slides` array of `playlist.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistEntry {
    /// Path to the `.vzglyd` archive or slide directory, relative to the slides directory.
    pub path: String,
    /// Set to `false` to skip this slide without removing it from the file. Absent means `true`.
    pub enabled: Option<bool>,
    /// Override display duration for this slide (seconds).
    pub duration_seconds: Option<u32>,
    /// Override transition-in for this slide.
    pub transition_in: Option<String>,
    /// Override transition-out for this slide.
    pub transition_out: Option<String>,
    /// Optional JSON parameters forwarded to the slide's configure buffer before init.
    pub params: Option<serde_json::Value>,
}

/// Load `playlist.json` from `slides_dir` if it exists.
///
/// Returns `Ok(None)` when the file is absent (not an error).
/// Returns `Err` when the file exists but cannot be read or parsed.
pub fn load_playlist(slides_dir: &Path) -> Result<Option<Playlist>, String> {
    let path = slides_dir.join(PLAYLIST_FILENAME);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let playlist: Playlist = serde_json::from_str(&content)
        .map_err(|e| format!("invalid {}: {e}", path.display()))?;
    Ok(Some(playlist))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join("VRX-64-playlist-tests")
            .join(format!(
                "{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock ok")
                    .as_nanos()
            ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn load_playlist_returns_none_when_file_absent() {
        let dir = temp_dir("absent");
        assert!(load_playlist(&dir).expect("no error").is_none());
    }

    #[test]
    fn load_playlist_parses_minimal_playlist() {
        let dir = temp_dir("minimal");
        fs::write(
            dir.join(PLAYLIST_FILENAME),
            r#"{"slides":[{"path":"clock.vzglyd"}]}"#,
        )
        .expect("write");

        let playlist = load_playlist(&dir).expect("ok").expect("some");
        assert_eq!(playlist.slides.len(), 1);
        assert_eq!(playlist.slides[0].path, "clock.vzglyd");
        assert!(playlist.slides[0].enabled.is_none());
        assert!(playlist.slides[0].duration_seconds.is_none());
    }

    #[test]
    fn load_playlist_parses_full_playlist() {
        let dir = temp_dir("full");
        fs::write(
            dir.join(PLAYLIST_FILENAME),
            r#"{
                "defaults": { "duration_seconds": 10, "transition_in": "crossfade" },
                "slides": [
                    { "path": "a.vzglyd", "duration_seconds": 20, "transition_out": "cut" },
                    { "path": "b.vzglyd", "enabled": false }
                ]
            }"#,
        )
        .expect("write");

        let playlist = load_playlist(&dir).expect("ok").expect("some");
        assert_eq!(playlist.defaults.duration_seconds, Some(10));
        assert_eq!(playlist.defaults.transition_in.as_deref(), Some("crossfade"));
        assert_eq!(playlist.slides[0].duration_seconds, Some(20));
        assert_eq!(playlist.slides[0].transition_out.as_deref(), Some("cut"));
        assert_eq!(playlist.slides[1].enabled, Some(false));
    }

    #[test]
    fn load_playlist_errors_on_invalid_json() {
        let dir = temp_dir("invalid");
        fs::write(dir.join(PLAYLIST_FILENAME), b"not json").expect("write");

        assert!(load_playlist(&dir).is_err());
    }
}
