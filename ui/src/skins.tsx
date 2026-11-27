import { useCallback, useEffect, useRef, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ArrowLeft, Ban, Check, CircleAlert, CirclePlus, Ellipsis, Eye, LoaderCircle, Pause, Pencil, Play, RefreshCw, Save, Search, Trash2, Undo2, UserRound, X } from 'lucide-react';
import type { AccountView } from './account-state';
import { DeleteLookDialog, SaveCurrentDialog, SkinDialog, type LookDraft } from './components/skin-dialogs';
import { CapePicture, SkinPicture } from './components/skin-picture';
import { SkinThumbnail } from './components/skin-thumbnails';
import { SkinViewer3D, prefersReducedMotion } from './components/skin-viewer';
import { TransientNotice, useTransientNotice } from './components/transient-notice';
import { Button } from './components/ui/button';
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from './components/ui/context-menu';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from './components/ui/dropdown-menu';
import { Hint } from './components/hint';
import { messageForError } from './errors';
import { dismissUnsaved, loadDismissed } from './skin-dismissed';
import { addLook, applySkins, deleteLook, editLook, loadSkins, saveCurrentLook, syncSkins } from './skin-api';
import {
  applyRequestOf, filterCapes, filterLooks, isDirty, noSelection, pickCape, pickLook, previewOf, syncedText, unsavedKey, validSelection, wornLookId,
  type CapeView, type LookView, type Selection, type SkinModel, type SkinsView,
} from './skin-state';
import { useTextureUrl } from './use-texture';
import { publishSkinsView } from './worn-skin';
import defaultSkin from './assets/default-skin.svg';

type Dialog = { kind: 'add' } | { kind: 'edit'; id: string } | { kind: 'save-current' } | { kind: 'delete'; id: string };
type Tab = 'skins' | 'capes';

/** Whether the selected account can use the manager: only authenticated Microsoft accounts can. */
export const canManageSkins = (account: AccountView | undefined) => account?.metadata.kind === 'microsoft';

/** The green check that marks what the account wears now. */
function WornMark({ label }: { label: string }) {
  return <span className="skin-card-check" role="img" aria-label={label}><Check aria-hidden="true" /></span>;
}

/** A card's right-click menu: the choice that opens a dialog keeps focus away from the card afterwards. */
function useDialogFocus() {
  const opensDialog = useRef(false);
  return {
    mark: () => { opensDialog.current = true; },
    onCloseAutoFocus: (event: Event) => { if (opensDialog.current) { event.preventDefault(); opensDialog.current = false; } },
  };
}

function LookCard({ look, active, selected, busy, onPick, onApply, onEdit, onDelete }: {
  look: LookView; active: boolean; selected: boolean; busy: boolean;
  onPick: () => void; onApply: () => void; onEdit: () => void; onDelete: () => void;
}) {
  const focus = useDialogFocus();
  return <li className="skin-card">
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div className="skin-card-body">
          <button type="button" className="skin-card-main" aria-pressed={selected} aria-label={look.name} onClick={onPick}>
            <span className="skin-card-art"><SkinPicture texture={look.texture} model={look.model} label={look.name} /></span>
            <span className="skin-card-name" title={look.name}>{look.name}</span>
          </button>
          {active && <WornMark label="Skin activa" />}
          <DropdownMenu>
            <DropdownMenuTrigger asChild><button type="button" className="skin-more" aria-label={`Más acciones para ${look.name}`}><Ellipsis aria-hidden="true" /></button></DropdownMenuTrigger>
            <DropdownMenuContent align="start">
              <DropdownMenuItem disabled={busy || active} onSelect={onApply}><Check />Aplicar</DropdownMenuItem>
              <DropdownMenuItem onSelect={onEdit}><Pencil />Editar</DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem className="text-destructive focus:text-destructive" onSelect={onDelete}><Trash2 />Eliminar</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent aria-label={`Acciones de ${look.name}`} onCloseAutoFocus={focus.onCloseAutoFocus}>
        <ContextMenuItem onSelect={onPick}><Eye />Vista previa</ContextMenuItem>
        <ContextMenuItem disabled={busy || active} onSelect={onApply}><Check />Aplicar</ContextMenuItem>
        <ContextMenuItem onSelect={() => { focus.mark(); onEdit(); }}><Pencil />Editar</ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem className="text-destructive focus:text-destructive" onSelect={() => { focus.mark(); onDelete(); }}><Trash2 />Eliminar</ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  </li>;
}

function CapeCard({ cape, worn, selected, busy, wearer, model, onPick, onApply }: {
  cape: CapeView; worn: boolean; selected: boolean; busy: boolean; wearer: string | null; model: SkinModel; onPick: () => void; onApply: () => void;
}) {
  return <li className="skin-card">
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div className="skin-card-body">
          <button type="button" className="skin-card-main" aria-pressed={selected} aria-label={cape.alias} onClick={onPick}>
            <span className="skin-card-art"><CapePicture id={cape.id} cached={cape.cached} wearer={wearer} model={model} label={cape.alias} /></span>
            <span className="skin-card-name" title={cape.alias}>{cape.alias}</span>
          </button>
          {worn && <WornMark label="Capa activa" />}
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent aria-label={`Acciones de ${cape.alias}`}>
        <ContextMenuItem onSelect={onPick}><Eye />Vista previa</ContextMenuItem>
        <ContextMenuItem disabled={busy || worn} onSelect={onApply}><Check />Aplicar</ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
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
  const [tab, setTab] = useState<Tab>('skins');
  const [query, setQuery] = useState('');
  const [playing, setPlaying] = useState(!prefersReducedMotion());
  const [elytra, setElytra] = useState(false);
  const [dismissed, setDismissed] = useState(loadDismissed);
  const { notice, show } = useTransientNotice();
  const current = useRef<string | null>(null);
  current.current = profileId;

  const accept = useCallback((next: SkinsView) => {
    if (next.profileId !== current.current && !(current.current === null)) return;
    publishSkinsView(next);
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
        if (live) { publishSkinsView(local); setView(local); }
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
  const choose = (next: Tab) => { setTab(next); setQuery(''); };
  const preview = view ? previewOf(view, selection) : null;
  const dirty = view ? isDirty(view, selection) : false;
  const skinImage = useTextureUrl('skin', preview?.texture ?? null);
  const previewCape = view?.capes.find(cape => cape.id === preview?.cape);
  const capeImage = useTextureUrl('cape', preview?.cape && previewCape?.cached ? preview.cape : null);
  const editing = dialog?.kind === 'edit' ? view?.library.find(look => look.id === dialog.id) : undefined;
  const deleting = dialog?.kind === 'delete' ? view?.library.find(look => look.id === dialog.id) : undefined;
  const dialogNotice = dialog ? notice : null;
  const looks = view ? filterLooks(view.library, query) : [];
  const capes = view ? filterCapes(view.capes, query) : [];
  const searching = query.trim() !== '';
  const reminder = view ? unsavedKey(view) : null;
  const showReminder = reminder !== null && !dismissed.has(reminder);
  const wornLook = view ? wornLookId(view) : null;

  const heading = <div className="page-heading">
    <Button variant="ghost" className="page-back h-auto p-0 hover:bg-transparent hover:text-foreground" onClick={onBack}><ArrowLeft aria-hidden="true" />Instancias</Button>
    <h1>Aspectos{account && profileId && <span className="page-heading-name"> · {account.metadata.username}</span>}</h1>
    <p>Personaliza tu personaje y cambia de estilo cuando quieras.</p>
  </div>;

  if (!desktop || !profileId) {
    return <>{heading}<section className="skins-empty" role="status"><UserRound aria-hidden="true" />
      <h2>{desktop ? 'Necesitas una cuenta de Microsoft' : 'Disponible en la aplicación de escritorio'}</h2>
      <p>{desktop ? 'Los aspectos solo se pueden gestionar con una cuenta de Microsoft. Las cuentas No-Premium no tienen un perfil en Minecraft.' : 'La gestión de aspectos está disponible en la aplicación de escritorio.'}</p>
    </section></>;
  }

  return <>
    {heading}
    {error && <div role="alert" className="skins-empty"><CircleAlert aria-hidden="true" /><p className="account-error">{error}</p><Button variant="secondary" onClick={() => setAttempt(value => value + 1)}>Reintentar</Button></div>}
    {!view && !error && <p className="skins-loading" role="status"><LoaderCircle className="spin" aria-hidden="true" />Cargando tus aspectos…</p>}
    {view && preview && <div className="skins-layout">
      <section className="skins-stage" aria-label="Vista previa">
        <div className="skins-viewer">
          {preview.custom
            ? <SkinViewer3D skin={skinImage.url} model={preview.model} cape={capeImage.url} playing={playing} elytra={elytra && !!capeImage.url} label={`Vista 3D de ${view.library.find(look => look.id === selection.lookId)?.name ?? 'tu skin'}`}
                fallback={<SkinThumbnail texture={preview.texture} model={preview.model} label="Tu skin" />} />
            : <div className="skins-default"><img src={defaultSkin} alt="" aria-hidden="true" /><p>Tu cuenta usa la skin predeterminada de Minecraft. Elige una de tu biblioteca o añade una nueva.</p></div>}
          {dirty && <span className="skins-pending">Vista previa</span>}
          {preview.custom && <div className="skins-tools">
            <Hint text={preview.cape ? 'Ver la capa como elytra' : 'Elige una capa para ver el elytra'}>
              <label className="inherit-toggle skins-elytra"><span>Elytra</span>
                <input role="switch" type="checkbox" checked={elytra && !!preview.cape} disabled={!preview.cape} onChange={event => setElytra(event.target.checked)} />
              </label>
            </Hint>
            <Hint text={playing ? 'Pausar la animación' : 'Reanudar la animación'}>
              <Button variant="secondary" size="icon" aria-pressed={!playing} aria-label={playing ? 'Pausar animación' : 'Reanudar animación'} onClick={() => setPlaying(value => !value)}>
                {playing ? <Pause aria-hidden="true" /> : <Play aria-hidden="true" />}
              </Button>
            </Hint>
          </div>}
          {dirty && <div className="skins-apply" role="group" aria-label="Cambios sin aplicar">
            <Button variant="outline" disabled={busy} onClick={() => setSelection(noSelection)}><Undo2 aria-hidden="true" />Descartar</Button>
            <Button disabled={busy} onClick={() => void apply()}>{busy ? <LoaderCircle className="spin" aria-hidden="true" /> : <Check aria-hidden="true" />}Aplicar</Button>
          </div>}
        </div>
      </section>

      <section className="skins-library" aria-label="Biblioteca">
        <div className="skins-toolbar">
          <div className="aspect-tabs" role="tablist" aria-label="Tipo de aspecto">
            <button type="button" role="tab" id="aspect-tab-skins" aria-selected={tab === 'skins'} aria-controls="aspect-panel" onClick={() => choose('skins')}>Skins<span className="aspect-count">{view.library.length}</span></button>
            <button type="button" role="tab" id="aspect-tab-capes" aria-selected={tab === 'capes'} aria-controls="aspect-panel" onClick={() => choose('capes')}>Capas<span className="aspect-count">{view.capes.length}</span></button>
          </div>
          <label className="skins-search"><Search aria-hidden="true" />
            <input type="search" value={query} placeholder={tab === 'skins' ? 'Buscar skin' : 'Buscar capa'} aria-label={tab === 'skins' ? 'Buscar skin' : 'Buscar capa'} autoComplete="off" onChange={event => setQuery(event.target.value)} />
          </label>
          <Hint text="Sincronizar con Minecraft"><Button variant="ghost" size="icon" aria-label="Sincronizar con Minecraft" disabled={syncing || busy} onClick={() => void sync(false)}>{syncing ? <LoaderCircle className="spin" aria-hidden="true" /> : <RefreshCw aria-hidden="true" />}</Button></Hint>
        </div>

        <div className="skins-scroll" role="tabpanel" id="aspect-panel" aria-labelledby={`aspect-tab-${tab}`}>
          {tab === 'skins' && showReminder && !searching && <div className="skins-unsaved" role="status">
            <span className="skin-card-art"><SkinPicture texture={view.current?.texture ?? null} model={view.current?.model ?? 'classic'} label="Tu aspecto actual" /></span>
            <div><strong>Aspecto actual sin guardar</strong><p>Añádelo a tu biblioteca para volver a usarlo cuando quieras.</p></div>
            <Button variant="secondary" disabled={busy} onClick={() => open({ kind: 'save-current' })}><Save aria-hidden="true" />Guardar</Button>
            <Hint text="No volver a avisar de este aspecto"><Button variant="ghost" size="icon" aria-label="Ocultar este aviso" onClick={() => reminder && setDismissed(previous => dismissUnsaved(previous, reminder))}><X aria-hidden="true" /></Button></Hint>
          </div>}
          {tab === 'skins' && <ul className="skin-grid">
            {!searching && <li className="skin-card"><ContextMenu><ContextMenuTrigger asChild><div className="skin-card-body">
              <button type="button" className="skin-card-main skin-card-new" disabled={busy} onClick={() => open({ kind: 'add' })}><CirclePlus aria-hidden="true" /><span className="skin-card-name">Nueva skin</span></button>
            </div></ContextMenuTrigger><ContextMenuContent aria-label="Acciones de Nueva skin"><ContextMenuItem disabled={busy} onSelect={() => open({ kind: 'add' })}><CirclePlus />Añadir skin</ContextMenuItem></ContextMenuContent></ContextMenu></li>}
            {looks.map(look => <LookCard key={look.id} look={look} busy={busy} active={wornLook === look.id} selected={selection.lookId === look.id}
              onPick={() => setSelection(pickLook(selection, look.id))}
              onApply={() => void apply({ lookId: look.id }, `«${look.name}» se aplicó a tu cuenta.`)}
              onEdit={() => open({ kind: 'edit', id: look.id })} onDelete={() => open({ kind: 'delete', id: look.id })} />)}
          </ul>}
          {tab === 'capes' && <ul className="skin-grid">
            {!searching && <li className="skin-card"><ContextMenu><ContextMenuTrigger asChild><div className="skin-card-body">
              <button type="button" className="skin-card-main skin-card-new" aria-pressed={preview.cape === null} aria-label="Sin capa" onClick={() => setSelection(pickCape(view, selection, null))}><Ban aria-hidden="true" /><span className="skin-card-name">Sin capa</span></button>
              {view.current?.cape == null && view.current && <WornMark label="Sin capa activa" />}
            </div></ContextMenuTrigger><ContextMenuContent aria-label="Acciones de Sin capa">
              <ContextMenuItem onSelect={() => setSelection(pickCape(view, selection, null))}><Eye />Vista previa</ContextMenuItem>
              <ContextMenuItem disabled={busy || view.current?.cape == null} onSelect={() => void apply({ lookId: null, capeChange: { id: null } }, 'Te quitaste la capa.')}><Check />Aplicar</ContextMenuItem>
            </ContextMenuContent></ContextMenu></li>}
            {capes.map(cape => <CapeCard key={cape.id} cape={cape} worn={view.current?.cape === cape.id} selected={preview.cape === cape.id} busy={busy}
              onApply={() => void apply({ lookId: null, capeChange: { id: cape.id } }, `La capa «${cape.alias}» se aplicó a tu cuenta.`)}
              wearer={view.current?.custom ? view.current.texture : null} model={view.current?.model ?? 'classic'} onPick={() => setSelection(pickCape(view, selection, cape.id))} />)}
          </ul>}
          {tab === 'skins' && view.library.length === 0 && !searching && <p className="skins-library-empty">Aún no has guardado ninguna skin. Elige «Nueva skin» para añadir un archivo PNG.</p>}
          {tab === 'capes' && view.capes.length === 0 && <p className="skins-library-empty">Tu cuenta no tiene capas.</p>}
          {searching && (tab === 'skins' ? looks.length === 0 : capes.length === 0) && <p className="skins-library-empty">Nada coincide con «{query.trim()}».</p>}
        </div>

        <p className="skins-sync" role="status">
          {syncFailed ? <><CircleAlert aria-hidden="true" />No pudimos sincronizar con Minecraft. Mostramos lo último guardado.</> : syncing ? <><LoaderCircle className="spin" aria-hidden="true" />Sincronizando con Minecraft…</> : syncedText(view.syncedMs)}
          {view.incomplete && !syncing && ' · Algunas imágenes se guardarán en la próxima sincronización.'}
        </p>
      </section>
    </div>}

    {view && dialog?.kind === 'add' && <SkinDialog view={view} busy={busy} error={dialogError} notice={dialogNotice} onCancel={() => open(null)} onSave={(draft, applyNow) => void saveLook(draft, applyNow)} />}
    {view && editing && <SkinDialog view={view} look={editing} busy={busy} error={dialogError} notice={dialogNotice} onCancel={() => open(null)} onSave={(draft, applyNow) => void saveLook(draft, applyNow)} />}
    {view && dialog?.kind === 'save-current' && <SaveCurrentDialog view={view} busy={busy} error={dialogError} notice={dialogNotice} onCancel={() => open(null)} onSave={name => void saveCurrent(name)} />}
    {deleting && <DeleteLookDialog look={deleting} busy={busy} error={dialogError} notice={dialogNotice} onCancel={() => open(null)} onConfirm={() => void remove(deleting.id)} />}
    {!dialog && <TransientNotice notice={notice} />}
  </>;
}
