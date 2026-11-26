import { useCallback, useEffect, useRef, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ArrowLeft, Check, CircleAlert, Ellipsis, LoaderCircle, Pencil, Plus, RefreshCw, RotateCw, Save, Trash2, Undo2, UserRound } from 'lucide-react';
import type { AccountView } from './account-state';
import { DeleteLookDialog, CapePicker, SaveCurrentDialog, SkinDialog, type LookDraft } from './components/skin-dialogs';
import { SkinThumbnail } from './components/skin-thumbnails';
import { SkinViewer3D, prefersReducedMotion, viewerAnimations, type ViewerAnimation } from './components/skin-viewer';
import { TransientNotice, useTransientNotice } from './components/transient-notice';
import { Button } from './components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from './components/ui/dropdown-menu';
import { Hint } from './components/hint';
import { messageForError } from './errors';
import { addLook, applySkins, deleteLook, editLook, loadSkins, saveCurrentLook, syncSkins } from './skin-api';
import {
  applyRequestOf, canSaveCurrent, isDirty, modelShortLabels, noSelection, pickCape, pickLook, previewOf, syncedText, validSelection,
  type LookView, type Selection, type SkinsView,
} from './skin-state';
import { useTextureUrl } from './use-texture';
import defaultSkin from './assets/default-skin.svg';

type Dialog = { kind: 'add' } | { kind: 'edit'; id: string } | { kind: 'save-current' } | { kind: 'delete'; id: string };

/** Whether the selected account can use the manager: only authenticated Microsoft accounts can. */
export const canManageSkins = (account: AccountView | undefined) => account?.metadata.kind === 'microsoft';

function LookCard({ look, view, selected, busy, onPick, onApply, onEdit, onDelete }: {
  look: LookView; view: SkinsView; selected: boolean; busy: boolean;
  onPick: () => void; onApply: () => void; onEdit: () => void; onDelete: () => void;
}) {
  const active = view.matchingLook === look.id;
  return <li className="skin-card">
    <button type="button" className="skin-card-main" aria-pressed={selected} onClick={onPick}>
      <SkinThumbnail texture={look.texture} model={look.model} label={look.name} />
      <span className="skin-card-name" title={look.name}>{look.name}</span>
      <span className="skin-card-meta">{modelShortLabels[look.model]}{look.cape ? ' · Con capa' : ''}</span>
    </button>
    {active && <span className="active-badge skin-card-badge"><Check aria-hidden="true" />Activa</span>}
    <DropdownMenu>
      <DropdownMenuTrigger asChild><button type="button" className="skin-more" aria-label={`Más acciones para ${look.name}`}><Ellipsis aria-hidden="true" /></button></DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem disabled={busy || active} onSelect={onApply}><Check />Aplicar a mi cuenta</DropdownMenuItem>
        <DropdownMenuItem onSelect={onEdit}><Pencil />Editar</DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem className="text-destructive focus:text-destructive" onSelect={onDelete}><Trash2 />Eliminar</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  </li>;
}

/**
 * The skin and cape manager. It works on the library kept on this computer and reaches Minecraft
 * Services only through the backend, which also refuses offline accounts, so this page is a
 * presentation of what the core allows rather than a gate of its own.
 */
export function SkinsPage({ account, onBack }: { account: AccountView | undefined; onBack: () => void }) {
  const profileId = canManageSkins(account) ? account?.metadata.profile_id ?? null : null;
  const desktop = isTauri();
  const [view, setView] = useState<SkinsView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [syncFailed, setSyncFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [selection, setSelection] = useState<Selection>(noSelection);
  const [busy, setBusy] = useState(false);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [dialogError, setDialogError] = useState<string | null>(null);
  const reduced = prefersReducedMotion();
  const [animation, setAnimation] = useState<ViewerAnimation>(reduced ? 'none' : 'idle');
  const [spin, setSpin] = useState(!reduced);
  const { notice, show } = useTransientNotice();
  const current = useRef<string | null>(null);
  current.current = profileId;

  const accept = useCallback((next: SkinsView) => {
    if (next.profileId !== current.current && !(current.current === null)) return;
    setView(next);
    setSelection(previous => validSelection(next, previous));
  }, []);

  const sync = useCallback(async (quiet: boolean) => {
    setSyncing(true);
    try {
      const next = await syncSkins();
      accept(next);
      setSyncFailed(false);
    } catch (cause) {
      setSyncFailed(true);
      if (!quiet) show(messageForError(cause), 'error');
    } finally {
      setSyncing(false);
    }
  }, [accept, show]);

  // Show what is on disk at once, then bring it up to date with the account.
  useEffect(() => {
    if (!profileId || !desktop) return;
    let live = true;
    setView(null); setError(null); setSelection(noSelection); setSyncFailed(false);
    void (async () => {
      try {
        const local = await loadSkins();
        if (live) setView(local);
      } catch (cause) {
        if (live) setError(messageForError(cause));
        return;
      }
      if (live) await sync(true);
    })();
    return () => { live = false; };
  }, [profileId, desktop, attempt, sync]);

  useEffect(() => {
    if (!profileId || !desktop) return;
    let active = true;
    let stop: (() => void) | undefined;
    void listen<SkinsView>('skins-changed', event => { if (active && event.payload.profileId === current.current) accept(event.payload); })
      .then(unlisten => { if (active) stop = unlisten; else unlisten(); });
    return () => { active = false; stop?.(); };
  }, [profileId, desktop, accept]);

  /** After a failure the backend may have changed things halfway; show what it now holds. */
  async function reload() {
    try { accept(await loadSkins()); } catch { /* the original error is what the player needs */ }
  }

  async function apply(request = view && applyRequestOf(view, selection), done = 'Cambios aplicados a tu cuenta.') {
    if (!request || busy) return;
    setBusy(true);
    try {
      accept(await applySkins(request));
      setSelection(noSelection);
      show(done);
    } catch (cause) {
      show(messageForError(cause), 'error');
      await reload();
    } finally {
      setBusy(false);
    }
  }

  async function saveLook(draft: LookDraft, applyNow: boolean) {
    if (!dialog || (dialog.kind !== 'add' && dialog.kind !== 'edit') || busy) return;
    setBusy(true); setDialogError(null);
    let saved: string;
    try {
      if (dialog.kind === 'add' && draft.png) {
        const result = await addLook({ name: draft.name, model: draft.model, cape: draft.cape, png: draft.png });
        accept(result.view);
        saved = result.id;
      } else if (dialog.kind === 'edit') {
        accept(await editLook(dialog.id, draft));
        saved = dialog.id;
      } else {
        throw 'invalid_data';
      }
    } catch (cause) {
      setDialogError(messageForError(cause));
      setBusy(false);
      return;
    }
    setDialog(null);
    if (!applyNow) { show('Skin guardada en tu biblioteca.'); setBusy(false); return; }
    try {
      accept(await applySkins({ lookId: saved }));
      setSelection(noSelection);
      show('Skin guardada y aplicada a tu cuenta.');
    } catch (cause) {
      show(`La skin se guardó, pero no pudimos aplicarla. ${messageForError(cause)}`, 'error');
      await reload();
    } finally {
      setBusy(false);
    }
  }

  async function saveCurrent(name: string) {
    if (busy) return;
    setBusy(true); setDialogError(null);
    try {
      accept((await saveCurrentLook(name)).view);
      setDialog(null);
      show('Tu aspecto actual se guardó en la biblioteca.');
    } catch (cause) {
      setDialogError(messageForError(cause));
    } finally {
      setBusy(false);
    }
  }

  async function remove(id: string) {
    if (busy) return;
    setBusy(true); setDialogError(null);
    try {
      accept(await deleteLook(id));
      setDialog(null);
      show('Eliminada de tu biblioteca.');
    } catch (cause) {
      setDialogError(messageForError(cause));
    } finally {
      setBusy(false);
    }
  }

  const open = (next: Dialog | null) => { setDialogError(null); setDialog(next); };
  const preview = view ? previewOf(view, selection) : null;
  const dirty = view ? isDirty(view, selection) : false;
  const skinImage = useTextureUrl('skin', preview?.texture ?? null);
  const previewCape = view?.capes.find(cape => cape.id === preview?.cape);
  const capeImage = useTextureUrl('cape', preview?.cape && previewCape?.cached ? preview.cape : null);
  const previewLook = view?.library.find(look => look.id === selection.lookId);
  const editing = dialog?.kind === 'edit' ? view?.library.find(look => look.id === dialog.id) : undefined;
  const deleting = dialog?.kind === 'delete' ? view?.library.find(look => look.id === dialog.id) : undefined;
  const dialogNotice = dialog ? notice : null;

  const heading = <div className="page-heading">
    <Button variant="ghost" className="page-back h-auto p-0 hover:bg-transparent hover:text-foreground" onClick={onBack}><ArrowLeft aria-hidden="true" />Instancias</Button>
    <h1>Skins y capas</h1>
    <p>{account && profileId ? `Cambia el aspecto de ${account.metadata.username} en Minecraft. Se aplica a tu cuenta de Microsoft y se verá en cualquier servidor.` : 'Cambia el aspecto de tu cuenta de Minecraft.'}</p>
  </div>;

  if (!desktop || !profileId) {
    return <>{heading}<section className="skins-empty" role="status"><UserRound aria-hidden="true" />
      <h2>{desktop ? 'Necesitas una cuenta de Microsoft' : 'Disponible en la aplicación de escritorio'}</h2>
      <p>{desktop ? 'Las skins y capas solo se pueden gestionar con una cuenta de Microsoft. Las cuentas No-Premium no tienen un perfil en Minecraft.' : 'La gestión de skins está disponible en la aplicación de escritorio.'}</p>
    </section></>;
  }

  return <>
    {heading}
    {error && <div role="alert" className="skins-empty"><CircleAlert aria-hidden="true" /><p className="account-error">{error}</p><Button variant="secondary" onClick={() => setAttempt(value => value + 1)}>Reintentar</Button></div>}
    {!view && !error && <p className="skins-loading" role="status"><LoaderCircle className="spin" aria-hidden="true" />Cargando tus skins…</p>}
    {view && preview && <div className="skins-layout">
      <section className="skins-stage" aria-labelledby="skins-preview-heading">
        <div className="skins-section-head">
          <h2 id="skins-preview-heading">{previewLook ? previewLook.name : 'Tu aspecto actual'}</h2>
          {dirty && <span className="skins-pending">Sin aplicar</span>}
        </div>
        <div className="skins-viewer">
          {preview.custom
            ? <SkinViewer3D skin={skinImage.url} model={preview.model} cape={capeImage.url} animation={animation} spin={spin} label={`Vista 3D de ${previewLook?.name ?? 'tu skin'}`}
                fallback={<SkinThumbnail texture={preview.texture} model={preview.model} label="Tu skin" />} />
            : <div className="skins-default"><img src={defaultSkin} alt="" aria-hidden="true" /><p>Tu cuenta usa la skin predeterminada de Minecraft. Elige una de tu biblioteca o añade una nueva.</p></div>}
        </div>
        {preview.custom && <div className="skins-viewer-controls">
          <div className="segmented" role="radiogroup" aria-label="Animación">
            {viewerAnimations.map(([value, label]) => <label key={value} className={animation === value ? 'is-selected' : undefined}>
              <input type="radio" name="skin-animation" value={value} checked={animation === value} disabled={reduced} onChange={() => setAnimation(value)} />{label}
            </label>)}
          </div>
          <Hint text={spin ? 'Detener el giro automático' : 'Girar automáticamente'}>
            <Button variant="secondary" size="icon" aria-pressed={spin} aria-label="Giro automático" disabled={reduced} onClick={() => setSpin(value => !value)}><RotateCw aria-hidden="true" /></Button>
          </Hint>
        </div>}
        <h3 className="skins-subheading">Capa</h3>
        <CapePicker name="Capa de tu cuenta" view={view} value={preview.cape} onChange={id => setSelection(pickCape(view, selection, id))} />
        {dirty && <div className="skins-apply" role="group" aria-label="Cambios sin aplicar">
          <Button variant="outline" disabled={busy} onClick={() => setSelection(noSelection)}><Undo2 aria-hidden="true" />Descartar</Button>
          <Button disabled={busy} onClick={() => void apply()}>{busy ? <LoaderCircle className="spin" aria-hidden="true" /> : <Check aria-hidden="true" />}Aplicar a mi cuenta</Button>
        </div>}
      </section>

      <section className="skins-library" aria-labelledby="skins-library-heading">
        <div className="skins-section-head">
          <h2 id="skins-library-heading">Mi biblioteca</h2>
          <div className="skins-actions">
            <Hint text="Sincronizar con Minecraft"><Button variant="secondary" size="icon" aria-label="Sincronizar con Minecraft" disabled={syncing || busy} onClick={() => void sync(false)}>{syncing ? <LoaderCircle className="spin" aria-hidden="true" /> : <RefreshCw aria-hidden="true" />}</Button></Hint>
            <Button disabled={busy} onClick={() => open({ kind: 'add' })}><Plus aria-hidden="true" />Añadir skin</Button>
          </div>
        </div>
        <p className="skins-sync" role="status">
          {syncFailed ? <><CircleAlert aria-hidden="true" />No pudimos sincronizar con Minecraft. Mostramos lo último guardado.</> : syncing ? <><LoaderCircle className="spin" aria-hidden="true" />Sincronizando con Minecraft…</> : syncedText(view.syncedMs)}
        </p>
        {view.incomplete && !syncing && <p className="skins-sync">Algunas imágenes no se pudieron guardar todavía. Se reintentará al sincronizar.</p>}
        {canSaveCurrent(view) && <div className="skins-unsaved" role="status">
          <SkinThumbnail texture={view.current?.texture ?? null} model={view.current?.model ?? 'classic'} label="Tu aspecto actual" />
          <div><strong>Tu aspecto actual no está guardado</strong><p>Guárdalo para volver a él cuando quieras, con su capa.</p></div>
          <Button variant="secondary" disabled={busy} onClick={() => open({ kind: 'save-current' })}><Save aria-hidden="true" />Guardar</Button>
        </div>}
        {view.library.length === 0
          ? <p className="skins-library-empty">Aún no has guardado ninguna skin. Añade un archivo PNG para empezar.</p>
          : <ul className="skin-grid">
            {view.library.map(look => <LookCard key={look.id} look={look} view={view} busy={busy} selected={selection.lookId === look.id}
              onPick={() => setSelection(pickLook(selection, look.id))}
              onApply={() => void apply({ lookId: look.id }, `«${look.name}» se aplicó a tu cuenta.`)}
              onEdit={() => open({ kind: 'edit', id: look.id })} onDelete={() => open({ kind: 'delete', id: look.id })} />)}
          </ul>}
      </section>
    </div>}

    {view && dialog?.kind === 'add' && <SkinDialog view={view} busy={busy} error={dialogError} notice={dialogNotice} onCancel={() => open(null)} onSave={(draft, applyNow) => void saveLook(draft, applyNow)} />}
    {view && editing && <SkinDialog view={view} look={editing} busy={busy} error={dialogError} notice={dialogNotice} onCancel={() => open(null)} onSave={(draft, applyNow) => void saveLook(draft, applyNow)} />}
    {view && dialog?.kind === 'save-current' && <SaveCurrentDialog view={view} busy={busy} error={dialogError} notice={dialogNotice} onCancel={() => open(null)} onSave={name => void saveCurrent(name)} />}
    {deleting && <DeleteLookDialog look={deleting} busy={busy} error={dialogError} notice={dialogNotice} onCancel={() => open(null)} onConfirm={() => void remove(deleting.id)} />}
    {!dialog && <TransientNotice notice={notice} />}
  </>;
}
