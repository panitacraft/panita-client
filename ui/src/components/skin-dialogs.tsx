import { useId, useRef, useState, type ChangeEvent, type ReactNode } from 'react';
import { Ban, Check, CirclePlus, Pencil, Save, Trash2, Upload, UserCheck, X } from 'lucide-react';
import { AccountDialog } from './account-dialog';
import { Hint } from './hint';
import { SkinViewer3D, prefersReducedMotion } from './skin-viewer';
import { CapeThumbnail, SkinThumbnail } from './skin-thumbnails';
import { TransientNotice, type TransientNoticeState } from './transient-notice';
import { Button } from './ui/button';
import { readSkinFile } from '../skin-api';
import { MAX_LOOK_NAME, lookNameProblem, modelLabels, skinFileProblem, suggestedLookName, type LookView, type SkinModel, type SkinsView } from '../skin-state';
import { useTextureUrl } from '../use-texture';

export type LookDraft = { name: string; model: SkinModel; cape: string | null; png: string | null };

/**
 * Choose no cape, or one the account owns, from a grid of square tiles. A tile carries no text: the cape's
 * name is its accessible name and tooltip, and the dialog names the chosen one beside the field label.
 * Capes that are not cached yet cannot be previewed.
 */
export function CapePicker({ view, value, onChange, name }: { view: SkinsView; value: string | null; onChange: (id: string | null) => void; name: string }) {
  const tile = (id: string | null, label: string, content: ReactNode) => <Hint key={id ?? 'none'} text={label} side="bottom">
    <button type="button" className="cape-tile" aria-label={label} aria-pressed={value === id} onClick={() => onChange(id)}>
      {content}{value === id && <span className="cape-tile-check" aria-hidden="true"><Check /></span>}
    </button>
  </Hint>;
  return <div className="cape-picker" role="group" aria-label={name}>
    {tile(null, 'Sin capa', <span className="cape-thumbnail"><Ban aria-hidden="true" /></span>)}
    {view.capes.map(cape => tile(cape.id, cape.alias, <CapeThumbnail id={cape.id} cached={cape.cached} label={cape.alias} />))}
    {view.capes.length === 0 && <p className="skin-hint">Tu cuenta no tiene capas.</p>}
  </div>;
}

export function ModelToggle({ value, onChange, name }: { value: SkinModel; onChange: (model: SkinModel) => void; name: string }) {
  const group = useId();
  return <div className="segmented" role="radiogroup" aria-label={name}>
    {(['classic', 'slim'] as const).map(model => {
      // "Clásico (Steve)" shows as "Clásico"; the nickname stays in the accessible name.
      const [short, nickname] = modelLabels[model].split(/ (?=\()/);
      return <label key={model} className={value === model ? 'is-selected' : undefined}>
        <input type="radio" name={group} value={model} checked={value === model} onChange={() => onChange(model)} />
        {short}{nickname && <span className="sr-only"> {nickname}</span>}
      </label>;
    })}
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
  // Adding starts with a dashed upload panel; once there is a file (or when editing) the 3D preview takes its place.
  const dropzone = adding && !file;
  const capeName = view.capes.find(item => item.id === cape)?.alias;
  const close = <button type="button" className="update-close" aria-label="Cerrar" disabled={busy} onClick={onCancel}><X aria-hidden="true" /></button>;
  return <AccountDialog title={<>{adding ? <CirclePlus className="dialog-title-icon" aria-hidden="true" /> : <Pencil className="dialog-title-icon" aria-hidden="true" />}{adding ? 'Añadir skin' : 'Editar skin'}</>}
    titleClassName="dialog-title" aside={close} className="skin-dialog" onCancel={() => { if (!busy) onCancel(); }}>
    <TransientNotice notice={notice} />
    <form noValidate onSubmit={event => { event.preventDefault(); submit(false); }}>
      <div className="skin-dialog-body">
        <div className="skin-dialog-side">
          <p className="sr-only" id={`${nameId}-file`}>Archivo de skin</p>
          <input ref={fileInput} type="file" accept="image/png,.png" className="sr-only" tabIndex={-1} aria-labelledby={`${nameId}-file`} onChange={event => void choose(event)} />
          {dropzone ? <div className="skin-dropzone">
            <Upload aria-hidden="true" />
            {fileError ? <p role="alert" className="skin-problem">{fileError}</p> : <p>PNG de 64×64 o 64×32 píxeles.</p>}
            <Button type="button" variant="outline" disabled={busy} onClick={() => fileInput.current?.click()}>Elegir archivo</Button>
          </div> : <div className="skin-dialog-preview">
            <SkinViewer3D skin={skinUrl} model={model} cape={capeImage.url} playing={!prefersReducedMotion()} label="Vista previa de la skin"
              fallback={<SkinThumbnail texture={look?.texture ?? null} model={model} label="Vista previa" />} />
            <div className="skin-dialog-file">
              <span className="skin-file-name">{file ? file.name : 'Se conserva la imagen actual'}</span>
              <Button type="button" variant="secondary" size="sm" disabled={busy} onClick={() => fileInput.current?.click()}>Cambiar archivo</Button>
            </div>
          </div>}
          {!dropzone && fileError && <p role="alert" className="skin-hint skin-problem">{fileError}</p>}
        </div>
        <div className="skin-dialog-fields">
          <label htmlFor={nameId}>Nombre</label>
          <input id={nameId} value={name} maxLength={MAX_LOOK_NAME} autoComplete="off" autoFocus onChange={event => setName(event.target.value)} aria-invalid={!!nameProblem} />
          {nameProblem && <p role="alert" className="skin-hint skin-problem">{nameProblem}</p>}

          <p className="skin-field-label">Brazos</p>
          <ModelToggle name="Modelo de brazos" value={model} onChange={setModel} />

          <p className="skin-field-label">Capa{capeName && <span className="skin-field-value"> · {capeName}</span>}</p>
          <CapePicker name="Capa de esta skin" view={view} value={cape} onChange={setCape} />
          {look?.cape && !owned && <p className="skin-hint">Esta skin tenía una capa que tu cuenta no tiene. Se quitará si guardas.</p>}
        </div>
      </div>
      {error && <p role="alert" className="account-error">{error}</p>}
      <div className="account-buttons">
        <Button type="button" variant="outline" disabled={busy} onClick={onCancel}><X aria-hidden="true" />Cancelar</Button>
        <Button type="submit" variant="secondary" disabled={!ready}><Save aria-hidden="true" />Guardar</Button>
        <Button type="button" disabled={!ready} onClick={() => submit(true)}><UserCheck aria-hidden="true" />Guardar y aplicar</Button>
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
