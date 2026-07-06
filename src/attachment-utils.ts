// Pure predicates for attachment/document filenames, shared by the
// Transactions and Pipeline views (previously duplicated in each).

// Same fixed set the Rust side recognizes — must stay in sync with
// read_attachment_data_url in src-tauri/src/extract.rs:image_mime_type().
export const IMAGE_EXTENSIONS = ['.jpg', '.jpeg', '.png', '.gif', '.webp'];

export function isImageFilename(filename: string): boolean {
    const lower = filename.toLowerCase();
    return IMAGE_EXTENSIONS.some((ext) => lower.endsWith(ext));
}

const ATTACHMENT_SUFFIX = '#attachment';

export function isImageAttachmentRef(ref: string): boolean {
    if (!ref.endsWith(ATTACHMENT_SUFFIX)) return false;
    return isImageFilename(ref.slice(0, -ATTACHMENT_SUFFIX.length));
}

export function attachmentFilename(ref: string): string {
    return ref.endsWith(ATTACHMENT_SUFFIX)
        ? ref.slice(0, -ATTACHMENT_SUFFIX.length)
        : ref;
}

export function isCsvDocument(filename: string): boolean {
    return filename.toLowerCase().endsWith('.csv');
}

export function isPdfDocument(filename: string): boolean {
    return filename.toLowerCase().endsWith('.pdf');
}
