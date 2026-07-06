import { useCallback, useState } from 'react';
import { readAttachmentDataUrl } from '../tauri-commands.ts';

// State + fetch logic for the shared image-attachment lightbox. Kept in its
// own module (not AttachmentLightbox.tsx) so that file only exports a
// component (react-refresh/only-export-components).
export function useAttachmentLightbox(ledgerPath: string | null) {
    const [src, setSrc] = useState<string | null>(null);
    const [filename, setFilename] = useState<string | null>(null);
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const openImage = useCallback(
        async (name: string) => {
            if (ledgerPath === null) return;
            setFilename(name);
            setSrc(null);
            setError(null);
            setLoading(true);
            try {
                const dataUrl = await readAttachmentDataUrl(ledgerPath, name);
                setSrc(dataUrl);
            } catch (e) {
                setError(String(e));
            } finally {
                setLoading(false);
            }
        },
        [ledgerPath],
    );

    const close = useCallback(() => {
        setSrc(null);
        setFilename(null);
        setError(null);
        setLoading(false);
    }, []);

    return { src, filename, loading, error, openImage, close };
}
