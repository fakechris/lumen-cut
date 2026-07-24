use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MusicTrack {
    #[serde(default = "default_music_track_id")]
    pub id: String,
    pub path: std::path::PathBuf,
    pub start: f64,
    pub end: f64,
    pub source_start: f64,
    pub volume: f64,
    pub fade_in: f64,
    pub fade_out: f64,
    pub ducking: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioMix {
    pub volume: f64,
    pub muted: bool,
    pub fade_in: f64,
    pub fade_out: f64,
    pub voice_enhance: bool,
    pub normalize_loudness: bool,
    pub loudness_target: f64,
    #[serde(default, deserialize_with = "deserialize_music_tracks")]
    pub music: Vec<MusicTrack>,
}

fn default_music_track_id() -> String {
    "music-1".into()
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PersistedMusicTracks {
    One(MusicTrack),
    Many(Vec<MusicTrack>),
}

fn deserialize_music_tracks<'de, D>(deserializer: D) -> Result<Vec<MusicTrack>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<PersistedMusicTracks>::deserialize(deserializer)?
        .map(|music| match music {
            PersistedMusicTracks::One(track) => vec![track],
            PersistedMusicTracks::Many(tracks) => tracks,
        })
        .unwrap_or_default())
}

impl Default for AudioMix {
    fn default() -> Self {
        Self {
            volume: 1.0,
            muted: false,
            fade_in: 0.0,
            fade_out: 0.0,
            voice_enhance: false,
            normalize_loudness: false,
            loudness_target: -16.0,
            music: Vec::new(),
        }
    }
}

impl AudioMix {
    pub fn fit_to_duration(&self, duration: f64) -> AppResult<Self> {
        if !self.volume.is_finite() || !(0.0..=2.0).contains(&self.volume) {
            return Err(AppError::Schema(
                "audio volume must be between 0% and 200%".into(),
            ));
        }
        if !self.fade_in.is_finite()
            || !self.fade_out.is_finite()
            || self.fade_in < 0.0
            || self.fade_out < 0.0
        {
            return Err(AppError::Schema(
                "audio fades must be finite nonnegative durations".into(),
            ));
        }
        if !self.loudness_target.is_finite() || !(-24.0..=-12.0).contains(&self.loudness_target) {
            return Err(AppError::Schema(
                "audio loudness target must be between -24 and -12 LUFS".into(),
            ));
        }
        let duration = duration.max(0.0);
        let fade_in = self.fade_in.min(duration);
        let fade_out = self.fade_out.min((duration - fade_in).max(0.0));
        let mut ids = std::collections::HashSet::new();
        let music = self
            .music
            .iter()
            .map(|track| {
                if !ids.insert(track.id.as_str()) {
                    return Err(AppError::Schema(format!(
                        "music track id {} is duplicated",
                        track.id
                    )));
                }
                track.fit_to_duration(duration)
            })
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Self {
            fade_in,
            fade_out,
            music,
            ..self.clone()
        })
    }

    pub fn validate(&self, duration: f64) -> AppResult<()> {
        if self.fit_to_duration(duration)? != *self {
            return Err(AppError::Schema(
                "audio fades must fit inside the edited duration".into(),
            ));
        }
        Ok(())
    }
}

impl MusicTrack {
    fn fit_to_duration(&self, duration: f64) -> AppResult<Self> {
        let finite = [
            self.start,
            self.end,
            self.source_start,
            self.volume,
            self.fade_in,
            self.fade_out,
        ]
        .into_iter()
        .all(f64::is_finite);
        if !finite
            || self.id.trim().is_empty()
            || self.path.as_os_str().is_empty()
            || self.start < 0.0
            || self.end <= self.start
            || self.source_start < 0.0
            || !(0.0..=2.0).contains(&self.volume)
            || self.fade_in < 0.0
            || self.fade_out < 0.0
        {
            return Err(AppError::Schema(
                "music track has invalid path, timing, volume, or fades".into(),
            ));
        }
        let start = self.start.min(duration);
        let end = self.end.min(duration);
        if end <= start {
            return Err(AppError::Schema(
                "music track must overlap the edited duration".into(),
            ));
        }
        let track_duration = end - start;
        let fade_in = self.fade_in.min(track_duration);
        let fade_out = self.fade_out.min((track_duration - fade_in).max(0.0));
        Ok(Self {
            start,
            end,
            fade_in,
            fade_out,
            ..self.clone()
        })
    }
}

pub fn load(project_dir: &Path) -> AppResult<AudioMix> {
    let path = project_dir.join("audio-mix.json");
    if !path.exists() {
        return Ok(AudioMix::default());
    }
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

pub fn save(project_dir: &Path, mix: &AudioMix, duration: f64) -> AppResult<()> {
    mix.validate(duration)?;
    crate::data::storage::write_json(&project_dir.join("audio-mix.json"), mix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_mix_round_trips_and_validates_export_safe_values() {
        let dir = tempfile::tempdir().unwrap();
        let mix = AudioMix {
            volume: 1.25,
            muted: false,
            fade_in: 0.5,
            fade_out: 1.0,
            voice_enhance: true,
            normalize_loudness: true,
            loudness_target: -16.0,
            music: vec![MusicTrack {
                id: "music-a".into(),
                path: "music.wav".into(),
                start: 1.0,
                end: 9.0,
                source_start: 2.0,
                volume: 0.25,
                fade_in: 0.5,
                fade_out: 1.0,
                ducking: true,
            }],
        };
        save(dir.path(), &mix, 10.0).unwrap();
        assert_eq!(load(dir.path()).unwrap(), mix);

        let legacy: AudioMix = serde_json::from_str(
            r#"{
                "volume": 1.0,
                "music": {
                    "path": "legacy.wav",
                    "start": 0.0,
                    "end": 2.0,
                    "sourceStart": 0.0,
                    "volume": 0.2,
                    "fadeIn": 0.0,
                    "fadeOut": 0.0,
                    "ducking": true
                }
            }"#,
        )
        .unwrap();
        assert_eq!(legacy.music.len(), 1);
        assert_eq!(legacy.music[0].id, "music-1");

        let multiple: AudioMix = serde_json::from_str(
            r#"{
                "music": [
                    {
                        "id": "music-a",
                        "path": "a.wav",
                        "start": 0.0,
                        "end": 2.0,
                        "sourceStart": 0.0,
                        "volume": 0.2,
                        "fadeIn": 0.0,
                        "fadeOut": 0.0,
                        "ducking": true
                    },
                    {
                        "id": "music-b",
                        "path": "b.wav",
                        "start": 2.0,
                        "end": 4.0,
                        "sourceStart": 0.0,
                        "volume": 0.2,
                        "fadeIn": 0.0,
                        "fadeOut": 0.0,
                        "ducking": false
                    }
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(multiple.music.len(), 2);

        assert!(AudioMix {
            volume: 2.1,
            ..Default::default()
        }
        .validate(10.0)
        .is_err());
        assert!(AudioMix {
            loudness_target: -10.0,
            ..Default::default()
        }
        .validate(10.0)
        .is_err());
        assert!(AudioMix {
            fade_in: 6.0,
            fade_out: 5.0,
            ..Default::default()
        }
        .validate(10.0)
        .is_err());
        assert_eq!(
            AudioMix {
                fade_in: 6.0,
                fade_out: 5.0,
                ..Default::default()
            }
            .fit_to_duration(10.0)
            .unwrap(),
            AudioMix {
                fade_in: 6.0,
                fade_out: 4.0,
                ..Default::default()
            }
        );
    }
}
