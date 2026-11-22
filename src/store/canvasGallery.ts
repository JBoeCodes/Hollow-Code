// Canvas gallery (overnight 10-11/12 F3). Since consolidation lane B
// (2026-10-13) it is Library → Results → Canvases (store/libraryMode.ts), not
// an overlay; these are the aliases `g x`, the palette and older links call.
import { closeLibraryCanvases, libraryCanvasesShown, openLibraryCanvases } from './libraryMode';
import type { GalleryCanvasRequest } from '../canvas/galleryRequest';

export const canvasGalleryOpen = libraryCanvasesShown;
/** Open the canvases, optionally filtered to one workspace. */
export const openCanvasGallery = (workspaceId = '', request: GalleryCanvasRequest | null = null) => openLibraryCanvases(workspaceId, request);
export const closeCanvasGallery = () => closeLibraryCanvases();
