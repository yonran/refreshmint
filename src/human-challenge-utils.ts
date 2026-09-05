export interface ChallengeViewport {
    width: number;
    height: number;
}

export interface DisplayRect {
    left: number;
    top: number;
    width: number;
    height: number;
}

/** Convert a pointer on the scaled stream image to Chrome viewport pixels. */
export function mapChallengePointer(
    clientX: number,
    clientY: number,
    rect: DisplayRect,
    viewport: ChallengeViewport,
): { x: number; y: number } | null {
    if (
        rect.width <= 0 ||
        rect.height <= 0 ||
        viewport.width <= 0 ||
        viewport.height <= 0
    ) {
        return null;
    }
    const relativeX = Math.min(Math.max(clientX - rect.left, 0), rect.width);
    const relativeY = Math.min(Math.max(clientY - rect.top, 0), rect.height);
    return {
        x: Math.round((relativeX / rect.width) * viewport.width),
        y: Math.round((relativeY / rect.height) * viewport.height),
    };
}
