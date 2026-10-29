import { Blocks } from 'lucide-react';
import type { CatalogInstance } from '../preview-catalog';
import { isPack } from '../library-state';
import minecraft from '../assets/brands/minecraft.png';
import neoforge from '../assets/brands/neoforge.png';
import fabric from '../assets/brands/fabric.png';
import quilt from '../assets/brands/quilt.svg';

const loaders: Record<string, { name: string; icon: string }> = {
  neoforge: { name: 'NeoForge', icon: neoforge },
  fabric: { name: 'Fabric', icon: fabric },
  quilt: { name: 'Quilt', icon: quilt },
};

export function InstanceBadges({ instance }: { instance: CatalogInstance }) {
  const pack = isPack(instance);
  const loaderId = instance.source?.loader ?? instance.installedSource?.loader;
  const loader = loaderId ? loaders[loaderId] : undefined;
  return <>
    {instance.minecraftVersion && <span><img className="game-icon" src={minecraft} alt="" aria-hidden="true" />Minecraft {instance.minecraftVersion}</span>}
    {pack && loaderId && <span>{loader ? <img className="loader-icon" src={loader.icon} alt="" aria-hidden="true" /> : <Blocks aria-hidden="true" />}{loader?.name ?? loaderId}</span>}
    {pack && (!loaderId || instance.source?.label) && <span>Modpack{instance.source?.label ? ` ${instance.source.label}` : ''}</span>}
  </>;
}
