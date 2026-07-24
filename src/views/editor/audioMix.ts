import type { AudioMix, MusicTrack } from "../../types";

export const DEFAULT_AUDIO_MIX: AudioMix = {
  volume: 1,
  muted: false,
  fadeIn: 0,
  fadeOut: 0,
  voiceEnhance: false,
  normalizeLoudness: false,
  loudnessTarget: -16,
  music: [],
};

export function audioGainAt(
  mix: AudioMix,
  sourceTime: number,
  duration: number,
): number {
  if (mix.muted) return 0;
  const fadeInGain = mix.fadeIn > 0 ? sourceTime / mix.fadeIn : 1;
  const remaining = Math.max(0, duration - sourceTime);
  const fadeOutGain = mix.fadeOut > 0 ? remaining / mix.fadeOut : 1;
  const fadeGain = Math.max(0, Math.min(1, fadeInGain, fadeOutGain));
  return Math.max(0, Math.min(1, mix.volume * fadeGain));
}

export function musicGainAt(
  track: MusicTrack,
  programTime: number,
  dialogueAudible: boolean,
): number {
  if (programTime < track.start || programTime >= track.end) return 0;
  const elapsed = programTime - track.start;
  const remaining = track.end - programTime;
  const fadeInGain = track.fadeIn > 0 ? elapsed / track.fadeIn : 1;
  const fadeOutGain = track.fadeOut > 0 ? remaining / track.fadeOut : 1;
  const duckGain = track.ducking && dialogueAudible ? 0.35 : 1;
  return Math.max(
    0,
    Math.min(1, track.volume * Math.min(1, fadeInGain, fadeOutGain) * duckGain),
  );
}
