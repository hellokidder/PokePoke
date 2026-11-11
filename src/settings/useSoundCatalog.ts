import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { CustomSoundInfo } from "./shared";

export function useSoundCatalog() {
  const [sounds, setSounds] = useState<string[]>([]);
  const [bundledSounds, setBundledSounds] = useState<string[]>([]);
  const [customSounds, setCustomSounds] = useState<CustomSoundInfo[]>([]);

  useEffect(() => {
    invoke<string[]>("list_system_sounds").then(setSounds);
    invoke<string[]>("list_bundled_sounds").then(setBundledSounds);
    void invoke<CustomSoundInfo[]>("list_custom_sounds")
      .then(setCustomSounds)
      .catch(() => setCustomSounds([]));
  }, []);

  const reloadCustomSounds = async () => {
    try {
      const next = await invoke<CustomSoundInfo[]>("list_custom_sounds");
      setCustomSounds(next);
    } catch {
      // 与首载一致:失败清空列表,不抛 unhandled rejection
      setCustomSounds([]);
    }
  };

  return { sounds, bundledSounds, customSounds, reloadCustomSounds };
}
