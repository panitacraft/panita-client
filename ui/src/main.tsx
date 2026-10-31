import React from 'react';
import { createRoot } from 'react-dom/client';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { ActivationGate, ActivationProvider } from './activation';
import { PlayProvider } from './play';
import { PreferencesProvider } from './settings';
import { GameLogWindow } from './game-output';
import { logWindowIdentity } from './game-output-state';
import { Launcher } from './launcher';
import { LauncherUpdateProvider } from './update';
import { applyMotion, readMotion } from './motion';
import './style.css';

applyMotion(readMotion());

const logIdentity = isTauri() ? logWindowIdentity(getCurrentWindow().label) : null;
createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {logIdentity ? <GameLogWindow {...logIdentity} /> : <PreferencesProvider><PlayProvider><ActivationProvider><ActivationGate><LauncherUpdateProvider><Launcher /></LauncherUpdateProvider></ActivationGate></ActivationProvider></PlayProvider></PreferencesProvider>}
  </React.StrictMode>,
);
