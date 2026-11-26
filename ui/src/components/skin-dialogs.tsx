import { useId, useRef, useState, type ChangeEvent } from 'react';
import { Ban, Check, FileUp, Save, Trash2, X } from 'lucide-react';
import { AccountDialog } from './account-dialog';
import { SkinViewer3D, prefersReducedMotion } from './skin-viewer';
import { CapeThumbnail, SkinThumbnail } from './skin-thumbnails';
import { TransientNotice, type TransientNoticeState } from './transient-notice';
import { Button } from './ui/button';
import { readSkinFile } from '../skin-api';
import { MAX_LOOK_NAME, lookNameProblem, modelLabels, skinFileProblem, suggestedLookName, type LookView, type SkinModel, type SkinsView } from '../skin-state';
import { useTextureUrl } from '../use-texture';

export type LookDraft = { name: string; model: SkinModel; cape: string | null; png: string | null };

/** Choose no cape, or one the account owns. Capes that are not cached yet cannot be previewed. */
export function CapePicker({ view, value, onChange, name }: { view: SkinsView; value: string | null; onChange: (id: string | null) => void; name: string }) {
  return <div className="cape-picker" role="group" aria-label={name}>
    <button type="button" className="cape-tile" aria-pressed={value === null} onClick={() => onChange(null)}>
      <span className="cape-thumbnail"><Ban aria-hidden="true" /></span><span>Sin capa</span>
    </button>
    {view.capes.map(cape => <button key={cape.id} type="button" className="cape-tile" aria-pressed={value === cape.id} onClick={() => onChange(cape.id)}>
      <CapeThumbnail id={cape.id} cached={cape.cached} label={cape.alias} /><span>{cape.alias}</span>
    </button>)}
    {view.capes.length === 0 && <p className="skin-hint">Tu cuenta no tiene capas.</p>}
  </div>;
}

export function ModelToggle({ value, onChange, name }: { value: SkinModel; onChange: (model: SkinModel) => void; name: string }) {
  const group = useId();
  return <div className="segmented" role="radiogroup" aria-label={name}>
    {(['classic', 'slim'] as const).map(model => <label key={model} className={value === model ? 'is-selected' : undefined}>
      <input type="radio" name={group} value={model} checked={value === model} onChange={() => onChange(model)} />
      {modelLabels[model]}
    </label>)}
  </div>;
}

/**
 * Add a skin from a PNG file or edit a saved one. The player sees the result in 3D while choosing the
 * file, the arm model and the cape, and can save it, or save it and put it on the account in one step.
 */
export function SkinDialog({ view, look, busy, error, notice, onCancel, onSave }: {
  view: SkinsView;
  /** Present when editing a saved look. */
  look?: LookView;
  busy: boolean;
  error: string | null;
  notice: TransientNoticeState | null;
  onCancel: () => void;
  onSave: (draft: LookDraft, apply: boolean) => void;
}) {
  const nameId = useId();
  const fileInput = useRef<HTMLInputElement>(null);
  const owned = look?.cape != null && view.capes.some(cape => cape.id === look.cape);
  const [name, setName] = useState(look?.name ?? suggestedLookName(view));
  const [model, setModel] = useState<SkinModel>(look?.model ?? 'classic');
  const [cape, setCape] = useState<string | null>(owned ? look?.cape ?? null : null);
  const [file, setFile] = useState<{ name: string; dataUrl: string; base64: string } | null>(null);
  const [fileError, setFileError] = useState<string | null>(null);
  const saved = useTextureUrl('skin', look?.texture ?? null);
  const capeCached = view.capes.find(item => item.id === cape)?.cached ?? false;
  const capeImage = useTextureUrl('cape', cape && capeCached ? cape : null);
  const adding = !look;
  const nameProblem = lookNameProblem(name);
  const ready = !nameProblem && !fileError && (!adding || file !== null) && !busy;
  const skinUrl = file?.dataUrl ?? saved.url;

  async function choose(event: ChangeEvent<HTMLInputElement>) {
    const chosen = event.target.files?.[0];
    event.target.value = '';
    if (!chosen) return;
    const problem = skinFileProblem(chosen);
    if (problem) { setFile(null); setFileError(problem); return; }
    try {
      setFile({ name: chosen.name, ...await readSkinFile(chosen) });
      setFileError(null);
    } catch {
      setFile(null); setFileError('No pudimos leer el archivo. Inténtalo de nuevo.');
    }
  }

  const submit = (apply: boolean) => { if (ready) onSave({ name: name.trim(), model, cape, png: file?.base64 ?? null }, apply); };
  return <AccountDialog title={adding ? 'Añadir skin' : 'Editar skin'} className="skin-dialog" onCancel={() => { if (!busy) onCancel(); }}>
    <TransientNotice notice={notice} />
    <form noValidate onSubmit={event => { event.preventDefault(); submit(false); }}>
      <div className="skin-dialog-body">
        <div className="skin-dialog-preview">
          <SkinViewer3D skin={skinUrl} model={model} cape={capeImage.url} playing={!prefersReducedMotion()} label="Vista previa de la skin"
            fallback={<SkinThumbnail texture={look?.texture ?? null} model={model} label="Vista previa" />} />
          {!skinUrl && <p className="skin-dialog-empty">Elige un archivo PNG para ver tu skin en 3D.</p>}
        </div>
        <div className="skin-dialog-fields">
          <label htmlFor={nameId}>Nombre</label>
          <input id={nameId} value={name} maxLength={MAX_LOOK_NAME} autoComplete="off" autoFocus onChange={event => setName(event.target.value)} aria-invalid={!!nameProblem} />
          {nameProblem && <p role="alert" className="skin-hint skin-problem">{nameProblem}</p>}

          <p className="skin-field-label" id={`${nameId}-file`}>Archivo de skin</p>
          <div className="skin-file">
            <input ref={fileInput} type="file" accept="image/png,.png" className="sr-only" tabIndex={-1} aria-labelledby={`${nameId}-file`} onChange={event => void choose(event)} />
            <Button type="button" variant="secondary" disabled={busy} onClick={() => fileInput.current?.click()}><FileUp aria-hidden="true" />{adding ? 'Elegir PNG…' : 'Cambiar PNG…'}</Button>
            <span className="skin-file-name">{file ? file.name : adding ? 'Ningún archivo' : 'Se conserva la imagen actual'}</span>
          </div>
          {fileError ? <p role="alert" className="skin-hint skin-problem">{fileError}</p> : <p className="skin-hint">PNG de 64×64 o 64×32 píxeles.</p>}

          <p className="skin-field-label">Modelo de brazos</p>
          <ModelToggle name="Modelo de brazos" value={model} onChange={setModel} />

          <p className="skin-field-label">Capa</p>
          <CapePicker name="Capa de esta skin" view={view} value={cape} onChange={setCape} />
          {look?.cape && !owned && <p className="skin-hint">Esta skin tenía una capa que tu cuenta no tiene. Se quitará si guardas.</p>}
        </div>
      </div>
      {error && <p role="alert" className="account-error">{error}</p>}
      <div className="account-buttons">
        <Button type="button" variant="outline" disabled={busy} onClick={onCancel}><X aria-hidden="true" />Cancelar</Button>
        <Button type="submit" variant="secondary" disabled={!ready}><Save aria-hidden="true" />Guardar</Button>
        <Button type="button" disabled={!ready} onClick={() => submit(true)}><Check aria-hidden="true" />Guardar y aplicar</Button>
      </div>
    </form>
  </AccountDialog>;
}

/** Give the look the account wears now a name and keep it in the library. */
export function SaveCurrentDialog({ view, busy, error, notice, onCancel, onSave }: {
  view: SkinsView;
  busy: boolean;
  error: string | null;
  notice: TransientNoticeState | null;
  onCancel: () => void;
  onSave: (name: string) => void;
}) {
  const nameId = useId();
  const [name, setName] = useState(suggestedLookName(view));
  const problem = lookNameProblem(name);
  return <AccountDialog title="Guardar en mi biblioteca" className="skin-save-dialog" onCancel={() => { if (!busy) onCancel(); }}>
    <TransientNotice notice={notice} />
    <form noValidate onSubmit={event => { event.preventDefault(); if (!problem && !busy) onSave(name.trim()); }}>
      <p>Guarda tu skin y tu capa actuales para volver a ellas cuando quieras, sin subir el archivo otra vez.</p>
      <label htmlFor={nameId}>Nombre</label>
      <input id={nameId} value={name} maxLength={MAX_LOOK_NAME} autoComplete="off" autoFocus onChange={event => setName(event.target.value)} aria-invalid={!!problem} />
      {problem && <p role="alert" className="skin-hint skin-problem">{problem}</p>}
      {error && <p role="alert" className="account-error">{error}</p>}
      <div className="account-buttons">
        <Button type="button" variant="outline" disabled={busy} onClick={onCancel}><X aria-hidden="true" />Cancelar</Button>
        <Button type="submit" disabled={!!problem || busy}><Save aria-hidden="true" />Guardar</Button>
      </div>
    </form>
  </AccountDialog>;
}

export function DeleteLookDialog({ look, busy, error, notice, onCancel, onConfirm }: {
  look: LookView;
  busy: boolean;
  error: string | null;
  notice: TransientNoticeState | null;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return <AccountDialog title="Eliminar de la biblioteca" className="skin-delete" onCancel={() => { if (!busy) onCancel(); }}>
    <TransientNotice notice={notice} />
    <p>«{look.name}» se quitará de tu biblioteca en este equipo. La skin que llevas puesta en Minecraft no cambia.</p>
    {error && <p role="alert" className="account-error">{error}</p>}
    <div className="account-buttons">
      <Button variant="secondary" autoFocus disabled={busy} onClick={onCancel}><X aria-hidden="true" />Cancelar</Button>
      <Button variant="destructive" disabled={busy} onClick={onConfirm}><Trash2 aria-hidden="true" />Eliminar</Button>
    </div>
  </AccountDialog>;
}
