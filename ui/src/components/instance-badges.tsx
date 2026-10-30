import { Hint } from './hint';
import { Blocks, RefreshCw } from 'lucide-react';
import type { CatalogInstance } from '../preview-catalog';
import { isPack } from '../library-state';
import minecraft from '../assets/brands/minecraft.svg';
import neoforge from '../assets/brands/neoforge.svg';
import forge from '../assets/brands/forge.svg';
import fabric from '../assets/brands/fabric.svg';
import quilt from '../assets/brands/quilt.svg';

const loaders: Record<string, { name: string; icon: string }> = {
  forge: { name: 'Forge', icon: forge },
  neoforge: { name: 'NeoForge', icon: neoforge },
  fabric: { name: 'Fabric', icon: fabric },
  quilt: { name: 'Quilt', icon: quilt },
};

export function InstanceBadges({ instance, updateAvailable = false, showLoaderHint = false }: { instance: CatalogInstance; updateAvailable?: boolean; showLoaderHint?: boolean }) {
  const pack = isPack(instance);
  const loaderId = instance.installedLoader?.name ?? instance.source?.loader ?? instance.installedSource?.loader;
  const loader = loaderId ? loaders[loaderId] : undefined;
  const loaderVersion = instance.installedLoader?.version;
  const label = instance.source?.label?.trim();
  const versionLabel = label ? `v${label.replace(/^v(?=\d)/i, '')}` : null;
  const loaderText = pack && loaderId ? (loader?.name ?? loaderId) + (loaderVersion ? ' ' + loaderVersion : '') : null;
  const gameBadge = <span tabIndex={loaderText && showLoaderHint ? 0 : undefined} aria-label={loaderText ? 'Minecraft ' + instance.minecraftVersion + ' · ' + loaderText : undefined}>
    <img className="game-icon" src={minecraft} alt="" aria-hidden="true" />
    {loaderText && (loader ? <img className="loader-icon" src={loader.icon} alt={loaderText} /> : <Blocks aria-label={loaderText} />)}
    Minecraft {instance.minecraftVersion}
  </span>;
  return <>
    {instance.minecraftVersion && (loaderText && showLoaderHint ? <Hint text={loaderText}>{gameBadge}</Hint> : gameBadge)}
    {pack && versionLabel && <span className="pack-version">{versionLabel}</span>}
    {updateAvailable && <span className="update-badge"><RefreshCw aria-hidden="true" />Actualización disponible</span>}
    {pack && !loaderId && !versionLabel && <span>Modpack</span>}
  </>;
}
