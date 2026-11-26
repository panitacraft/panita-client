/**
 * The angles every skin is shown at, in radians of turn around the vertical axis. The 3D player
 * starts at `DEFAULT_YAW` and so do the saved skins in the library, so a card and the preview agree.
 * Zero faces the viewer; a negative turn brings the face toward the left of the screen.
 */
export const DEFAULT_YAW = -0.5;
/** A cape is judged from behind, turned a little the other way so its folds show. */
export const CAPE_YAW = Math.PI + 0.45;
