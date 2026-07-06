import { describe, expect, it } from 'vitest';
import {
    attachmentFilename,
    isCsvDocument,
    isImageAttachmentRef,
    isImageFilename,
    isPdfDocument,
} from './attachment-utils.ts';

describe('isImageFilename', () => {
    it('is true for a plain image filename', () => {
        expect(isImageFilename('receipt.png')).toBe(true);
    });

    it('is case-insensitive', () => {
        expect(isImageFilename('RECEIPT.PNG')).toBe(true);
    });

    it('is false for a pdf', () => {
        expect(isImageFilename('statement.pdf')).toBe(false);
    });
});

describe('isImageAttachmentRef', () => {
    it('is true for an image ref with the #attachment suffix', () => {
        expect(isImageAttachmentRef('receipt.png#attachment')).toBe(true);
    });

    it('is false for a pdf attachment ref', () => {
        expect(isImageAttachmentRef('statement.pdf#attachment')).toBe(false);
    });

    it('is false for a plain image filename without the suffix', () => {
        expect(isImageAttachmentRef('receipt.png')).toBe(false);
    });

    it('is case-insensitive on the extension', () => {
        expect(isImageAttachmentRef('receipt.PNG#attachment')).toBe(true);
    });
});

describe('attachmentFilename', () => {
    it('strips the #attachment suffix', () => {
        expect(attachmentFilename('receipt.png#attachment')).toBe(
            'receipt.png',
        );
    });

    it('returns the ref unchanged when there is no suffix', () => {
        expect(attachmentFilename('receipt.png')).toBe('receipt.png');
    });
});

describe('isCsvDocument', () => {
    it('matches .csv case-insensitively', () => {
        expect(isCsvDocument('rows.CSV')).toBe(true);
        expect(isCsvDocument('statement.pdf')).toBe(false);
    });
});

describe('isPdfDocument', () => {
    it('matches .pdf case-insensitively', () => {
        expect(isPdfDocument('statement.PDF')).toBe(true);
        expect(isPdfDocument('rows.csv')).toBe(false);
    });
});
