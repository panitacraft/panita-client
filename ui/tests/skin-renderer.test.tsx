import { beforeEach, expect, test, vi } from 'vitest';

type Render = { skin: string; model: string; cape: string | null; yaw: number };
const fake = vi.hoisted(() => ({ created: 0, renders: [] as Render[], frames: [] as { width: number; height: number; zoom: number; lift: number; fov: number; pitch: number; spread: number }[], active: 0, overlap: false }));

// A stand-in for the 3D library: it records what was drawn and notices two renders at once.
vi.mock('skinview3d', () => ({
  SkinViewer: class {
    canvas = { toDataURL: () => `picture-${fake.renders.length}` };
    playerWrapper = { rotation: { y: 0 }, position: { y: 0 } };
    width = 0; height = 0; zoom = 0; fov = 0;
    cameraY = 0; cameraZ = 1;
    camera = { position: { set: (_x: number, y: number, z: number) => { this.cameraY = y; this.cameraZ = z; } }, lookAt: () => {} };
    playerObject = { skin: { leftArm: { rotation: { z: 0 } }, rightArm: { rotation: { z: 0 } } } };
    setSize(width: number, height: number) { this.width = width; this.height = height; }
    animation: unknown = 'unset';
    skin = ''; model = ''; cape: string | null = null;
    constructor() { fake.created += 1; }
    loadSkin(image: { name: string }, options: { model: string }) { this.skin = image.name; this.model = options.model; }
    loadCape(image: { name: string }) { this.cape = image.name; }
    resetCape() { this.cape = null; }
    render() {
      fake.active += 1;
      if (fake.active > 1) fake.overlap = true;
      fake.frames.push({ width: this.width, height: this.height, zoom: this.zoom, lift: this.playerWrapper.position.y, fov: this.fov, pitch: Math.atan2(this.cameraY, this.cameraZ), spread: this.playerObject.skin.leftArm.rotation.z });
      fake.renders.push({ skin: this.skin, model: this.model, cape: this.cape, yaw: this.playerWrapper.rotation.y });
      fake.active -= 1;
    }
  },
}));
vi.mock('../src/use-texture', () => ({
  decodeImage: async (url: string) => {
    await new Promise(resolve => setTimeout(resolve, url.length % 3));
    if (url === 'broken') throw new Error('unreadable');
    return { name: url };
  },
}));

const load = () => import('../src/skin-renderer');
const request = (key: string, extra: Partial<{ skin: string; model: 'classic' | 'slim'; cape: string | null; yaw: number; frame: { width: number; height: number; zoom: number; lift: number; pitch?: number; spread?: number; fov?: number } }> = {}) =>
  ({ key, skin: `skin-${key}`, model: 'classic' as const, cape: null, yaw: -0.5, ...extra });

beforeEach(() => {
  vi.resetModules();
  fake.created = 0; fake.renders = []; fake.frames = []; fake.active = 0; fake.overlap = false;
});

test('one hidden player draws every picture, in turn, with the asked model, cape and angle', async () => {
  const { renderThumbnail } = await load();
  const pictures = await Promise.all([
    renderThumbnail(request('a')),
    renderThumbnail(request('b', { model: 'slim', yaw: 3.5, cape: 'cape-b' })),
    renderThumbnail(request('c')),
  ]);
  expect(fake.created).toBe(1);
  expect(fake.overlap).toBe(false);
  expect(fake.renders).toEqual([
    { skin: 'skin-a', model: 'default', cape: null, yaw: -0.5 },
    { skin: 'skin-b', model: 'slim', cape: 'cape-b', yaw: 3.5 },
    { skin: 'skin-c', model: 'default', cape: null, yaw: -0.5 },
  ]);
  expect(new Set(pictures).size).toBe(3);
});

test('a picture is drawn once however many cards ask for it, and is then served from memory', async () => {
  const { renderThumbnail, cachedThumbnail } = await load();
  expect(cachedThumbnail('a')).toBeUndefined();
  const [first, second] = await Promise.all([renderThumbnail(request('a')), renderThumbnail(request('a'))]);
  expect(first).toBe(second);
  expect(fake.renders).toHaveLength(1);
  expect(cachedThumbnail('a')).toBe(first);
  await renderThumbnail(request('a'));
  expect(fake.renders).toHaveLength(1);
  // The same skin at another angle or on another model is another picture.
  await renderThumbnail(request('a-slim', { model: 'slim' }));
  expect(fake.renders).toHaveLength(2);
});

test('a picture that cannot be drawn fails alone, is not remembered, and the others still come', async () => {
  const { renderThumbnail, cachedThumbnail } = await load();
  const results = await Promise.allSettled([renderThumbnail(request('x', { skin: 'broken' })), renderThumbnail(request('y'))]);
  expect(results.map(result => result.status)).toEqual(['rejected', 'fulfilled']);
  expect(cachedThumbnail('x')).toBeUndefined();
  // Asking again tries again instead of replaying the failure.
  expect(await renderThumbnail(request('x'))).toMatch(/^picture-/);
  expect(cachedThumbnail('x')).toBeDefined();
});

test('memory stays bounded: the oldest pictures are forgotten first', async () => {
  const { renderThumbnail, cachedThumbnail } = await load();
  for (let index = 0; index < 250; index += 1) await renderThumbnail(request(`k${index}`));
  expect(cachedThumbnail('k0')).toBeUndefined();
  expect(cachedThumbnail('k249')).toBeDefined();
});

test('a bust is framed on the head, and the next full-body picture goes back to the whole body', async () => {
  const { renderThumbnail, BUST_FRAME } = await load();
  await renderThumbnail(request('whole-1'));
  await renderThumbnail(request('bust-1', { frame: BUST_FRAME }));
  await renderThumbnail(request('whole-2'));
  const { pitch, spread, fov, ...framing } = fake.frames[1];
  expect(framing).toEqual({ width: BUST_FRAME.width, height: BUST_FRAME.height, zoom: BUST_FRAME.zoom, lift: BUST_FRAME.lift });
  expect(fake.frames[0]).not.toEqual(fake.frames[1]);
  expect(fake.frames[2]).toEqual(fake.frames[0]);
  // Seen from above with the arms angled out, and a narrower lens; the body pictures are level with straight arms.
  expect(pitch).toBeCloseTo(BUST_FRAME.pitch ?? 0);
  expect(spread).toBeCloseTo(BUST_FRAME.spread ?? 0);
  expect(fov).toBe(BUST_FRAME.fov);
  expect(fake.frames[0].pitch).toBeCloseTo(0);
  expect(fake.frames[0].spread).toBe(0);
  expect(fake.frames[0].fov).toBe(40);
  // Head and shoulders are a closer, lower-centred view than the whole body.
  expect(BUST_FRAME.zoom).toBeGreaterThan(fake.frames[0].zoom);
  expect(BUST_FRAME.lift).toBeLessThan(fake.frames[0].lift);
});
