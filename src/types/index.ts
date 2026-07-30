export interface SubtitleEvent {
  id: string;
  start: number;
  end: number;
  text: string;
  speaker?: string;
  character?: string;
  source: "ocr" | "asr" | "manual";
  confidence: number;
}

export interface Track {
  type: "subtitle" | "voice" | "translation";
  events: SubtitleEvent[];
}

export interface Project {
  path: string;
  video: string;
  name: string;
  tracks: Track[];
  created_at: string;
  updated_at: string;
}

export interface RecentProject {
  path: string;
  name: string;
  updated_at: string;
}

export interface VideoMetadata {
  path: string;
  duration: number;
  width: number;
  height: number;
  fps: number;
  codec: string;
  audio_codec: string | null;
}
