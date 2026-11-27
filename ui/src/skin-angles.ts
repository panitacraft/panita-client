/**
 * The angles every skin is shown at, in radians of turn around the vertical axis. The 3D player
 * starts at `DEFAULT_YAW` and so do the saved skins in the library, so a card and the preview agree.
 * Zero faces the viewer; a positive turn brings the face toward the right of the screen.
 */
export const DEFAULT_YAW = 0.5;
/** A cape is judged from behind, turned a little so its folds show, the same way round as the skins. */
export const CAPE_YAW = Math.PI - 0.45;
/** The bust beside an account, turned a little less than the full-body views, as the online busts it replaces are. */
export const BUST_YAW = 0.6;
