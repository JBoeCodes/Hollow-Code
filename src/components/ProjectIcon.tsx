import { createSignal, Show } from 'solid-js';
import { Icon } from '../icons';
import { workspaceIcons, workspaces } from '../store/workspaces';
import { projectColorPicks } from '../store/projectColors';
import { monogram, projectColor } from '../projectIconColors';
import { createIconSrc } from '../remoteImageLoader';
import '../styles/project-icon.css';

/** A workspace's identity mark: the project's own app artwork when it has
 *  one (with its GitHub owner as a small badge), else the owner's avatar
 *  (organization or personal), else its monogram on its color (4.4). Broken or offline
 *  images fall back a step instead of showing a broken-image glyph. */
export function ProjectIcon(props: { workspaceId: string; size?: 'rail' | 'card' }) {
  const icon = () => workspaceIcons()[props.workspaceId];
  const [imageFailed, setImageFailed] = createSignal(false);
  const [avatarFailed, setAvatarFailed] = createSignal(false);
  const imageSrc = createIconSrc(() => icon()?.imageUrl ?? null);
  const image = () => (imageFailed() ? null : imageSrc());
  // The owner's GitHub avatar is remote; the main window's CSP allows no
  // remote images, so it comes through the backend (`createIconSrc`), and is
  // null while it loads or after it fails.
  const avatarSrc = createIconSrc(() => icon()?.avatarUrl ?? null);
  const avatar = () => (avatarFailed() ? null : avatarSrc());
  const owner = () => icon()?.owner ?? null;
  const title = () => workspaces().find(w => w.id === props.workspaceId)?.title ?? '';
  const color = () => projectColor(props.workspaceId, projectColorPicks()[props.workspaceId]);
  return <span class="project-icon" data-size={props.size ?? 'rail'} title={owner() ? `GitHub · ${owner()}` : undefined} aria-hidden="true">
    <Show when={image()} fallback={<Show when={avatar()} fallback={<Show when={title()} fallback={<Icon name="folder" size={props.size === 'card' ? 20 : 15} />}>
      <span class="project-icon-mono" style={{ '--project-hue': String(color().hue) }}>{monogram(title())}</span></Show>}>
      {src => <img class="project-icon-avatar" src={src()} alt="" onError={() => setAvatarFailed(true)} />}
    </Show>}>
      {src => <>
        <img class="project-icon-art" src={src()} alt="" onError={() => setImageFailed(true)} />
        <Show when={avatar()}>{badge => <img class="project-icon-badge" src={badge()} alt="" onError={() => setAvatarFailed(true)} />}</Show>
      </>}
    </Show>
  </span>;
}
