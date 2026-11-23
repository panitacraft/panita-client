import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Screenshot, ScreenshotAction, ScreenshotList } from './screenshot-state';

const commands: Record<ScreenshotAction, string> = { open: 'open_screenshot', reveal: 'reveal_screenshot', copy: 'copy_screenshot' };

/** The player's screenshots, newest first. Empty outside the app. */
export function loadScreenshots(instanceId: string): Promise<ScreenshotList> {
  if (!isTauri()) return Promise.resolve({ screenshots: [], truncated: false });
  return invoke<ScreenshotList>('list_screenshots', { instanceId });
}

/**
 * The image as a data URL. The backend validates the name and returns raw bytes; a data
 * URL keeps the content security policy unchanged and needs no cleanup afterwards.
 */
export async function loadScreenshotImage(instanceId: string, screenshot: Screenshot): Promise<string> {
  const bytes = await invoke<ArrayBuffer>('screenshot_image', { instanceId, name: screenshot.name });
  return new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => typeof reader.result === 'string' ? resolve(reader.result) : reject(new Error('unreadable image'));
    reader.onerror = () => reject(reader.error ?? new Error('unreadable image'));
    reader.readAsDataURL(new Blob([bytes], { type: 'image/png' }));
  });
}

export async function runScreenshotAction(action: ScreenshotAction, instanceId: string, screenshot: Screenshot): Promise<void> {
  await invoke(commands[action], { instanceId, name: screenshot.name });
}
