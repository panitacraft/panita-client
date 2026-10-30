import { Blocks } from 'lucide-react';
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

export function InstanceBadges({ instance }: { instance: CatalogInstance }) {
  const pack = isPack(instance);
  const loaderId = instance.installedLoader?.name ?? instance.source?.loader ?? instance.installedSource?.loader;
  const loader = loaderId ? loaders[loaderId] : undefined;
  const loaderVersion = instance.installedLoader?.version;
  const label = instance.source?.label?.trim();
  const versionLabel = label ? `v${label.replace(/^v(?=\d)/i, '')}` : null;
  return <>
    {instance.minecraftVersion && <span><img className="game-icon" src={minecraft} alt="" aria-hidden="true" />Minecraft {instance.minecraftVersion}</span>}
    {pack && loaderId && <span title={loaderVersion ? 'Loader de la instalación local' : undefined}>{loader ? <img className="loader-icon" src={loader.icon} alt="" aria-hidden="true" /> : <Blocks aria-hidden="true" />}{loader?.name ?? loaderId}{loaderVersion ? ` ${loaderVersion}` : ''}</span>}
    {pack && versionLabel && <span className="pack-version">{versionLabel}</span>}
    {pack && !loaderId && !versionLabel && <span>Modpack</span>}
  </>;
}
