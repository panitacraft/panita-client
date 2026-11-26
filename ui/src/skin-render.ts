import type { SkinModel } from './skin-state';

/**
 * The flat, front-facing picture of a skin that the library shows on its cards. It is cut out
 * of the skin texture in a fixed 16 by 32 pixel frame, the way the official launcher does.
 * Coordinates are in the 64 by 64 layout; larger skins scale them.
 */
export const FRONT_WIDTH = 16;
export const FRONT_HEIGHT = 32;

export type Part = { sx: number; sy: number; w: number; h: number; dx: number; dy: number; flip: boolean };

/** A skin that is twice as wide as it is tall uses the old layout, with no left limbs of its own. */
export const isLegacyLayout = (width: number, height: number) => height * 2 === width;

/** Parts in drawing order: the base layer first, then the outer layer over it. */
export function frontParts(model: SkinModel, legacy: boolean): Part[] {
  const arm = model === 'slim' ? 3 : 4;
  const armX = model === 'slim' ? 1 : 0;
  const part = (sx: number, sy: number, w: number, h: number, dx: number, dy: number, flip = false): Part => ({ sx, sy, w, h, dx, dy, flip });
  const base: Part[] = [
    part(8, 8, 8, 8, 4, 0),
    part(20, 20, 8, 12, 4, 8),
    part(44, 20, arm, 12, armX, 8),
    legacy ? part(44, 20, arm, 12, 12, 8, true) : part(36, 52, arm, 12, 12, 8),
    part(4, 20, 4, 12, 4, 20),
    legacy ? part(4, 20, 4, 12, 8, 20, true) : part(20, 52, 4, 12, 8, 20),
  ];
  const outer: Part[] = [part(40, 8, 8, 8, 4, 0)];
  if (!legacy) {
    outer.push(
      part(20, 36, 8, 12, 4, 8),
      part(44, 36, arm, 12, armX, 8),
      part(52, 52, arm, 12, 12, 8),
      part(4, 36, 4, 12, 4, 20),
      part(4, 52, 4, 12, 8, 20),
    );
  }
  return [...base, ...outer];
}

type Source = { width: number; height: number };

/** Draw the front of a skin into a canvas context sized `FRONT_WIDTH` by `FRONT_HEIGHT`. */
export function drawFront(context: CanvasRenderingContext2D, image: CanvasImageSource & Source, model: SkinModel): void {
  context.clearRect(0, 0, FRONT_WIDTH, FRONT_HEIGHT);
  context.imageSmoothingEnabled = false;
  const scale = image.width / 64;
  for (const { sx, sy, w, h, dx, dy, flip } of frontParts(model, isLegacyLayout(image.width, image.height))) {
    context.save();
    if (flip) { context.translate(dx + w, dy); context.scale(-1, 1); } else context.translate(dx, dy);
    context.drawImage(image, sx * scale, sy * scale, w * scale, h * scale, 0, 0, w, h);
    context.restore();
  }
}

export const CAPE_WIDTH = 10;
export const CAPE_HEIGHT = 16;

/** The outside of a cape, the face other players see from behind. The old 22 by 17 layout has its own scale. */
export function capeSource({ width, height }: Source): { x: number; y: number; w: number; h: number } {
  const scale = height * 22 === width * 17 ? width / 22 : width / 64;
  return { x: scale, y: scale, w: CAPE_WIDTH * scale, h: CAPE_HEIGHT * scale };
}

export function drawCape(context: CanvasRenderingContext2D, image: CanvasImageSource & Source): void {
  context.clearRect(0, 0, CAPE_WIDTH, CAPE_HEIGHT);
  context.imageSmoothingEnabled = false;
  const { x, y, w, h } = capeSource(image);
  context.drawImage(image, x, y, w, h, 0, 0, CAPE_WIDTH, CAPE_HEIGHT);
}
