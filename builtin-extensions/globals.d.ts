interface PageDownload {
    path: string;
    suggestedFilename?: string;
    [key: string]: unknown;
}

interface PageSnapshotOptions {
    incremental?: boolean;
    track?: string;
}

interface ByRoleOptions {
    name?: string | RegExp;
    exact?: boolean;
    checked?: boolean;
    disabled?: boolean;
    expanded?: boolean;
    includeHidden?: boolean;
    level?: number;
    pressed?: boolean;
    selected?: boolean;
}

interface Locator {
    readonly selector: string;
    locator(selector: string): Locator;
    getByRole(role: string, options?: ByRoleOptions): Locator;
    first(): Locator;
    last(): Locator;
    nth(index: number): Locator;
    count(): Promise<number>;
    click(options?: { timeout?: number } | number): Promise<void>;
    fill(value: string, options?: { timeout?: number } | number): Promise<void>;
    innerText(options?: { timeout?: number } | number): Promise<string>;
    textContent(options?: { timeout?: number } | number): Promise<string>;
    getAttribute(
        name: string,
        options?: { timeout?: number } | number,
    ): Promise<string>;
    inputValue(options?: { timeout?: number } | number): Promise<string>;
    isVisible(): Promise<boolean>;
    isEnabled(): Promise<boolean>;
    wait_for(options?: {
        state?: 'attached' | 'detached' | 'visible' | 'hidden';
        timeout?: number;
    }): Promise<void>;
}

interface PageApi {
    locator(selector: string): Locator;
    getByRole(role: string, options?: ByRoleOptions): Locator;
    goto(url: string): Promise<void>;
    url(): Promise<string>;
    reload(): Promise<void>;
    waitForSelector(selector: string, timeoutMs?: number): Promise<void>;
    waitForNavigation(timeoutMs?: number): Promise<void>;
    waitForURL(pattern: string, timeoutMs?: number): Promise<void>;
    waitForLoadState(
        state?: 'load' | 'domcontentloaded' | 'networkidle',
        timeoutMs?: number,
    ): Promise<void>;
    waitForResponse(urlPattern: string, timeoutMs?: number): Promise<string>;
    networkRequests(): Promise<string>;
    responsesReceived(): Promise<string>;
    clearNetworkRequests(): Promise<void>;
    waitForPopup(timeoutMs?: number): Promise<PageApi>;
    waitForEvent(event: 'popup', timeoutMs?: number): Promise<PageApi>;
    /** @deprecated Removed. Use browser.pages(). */
    tabs(): Promise<never>;
    /** @deprecated Removed. Use browser.pages() and direct Page handles. */
    selectTab(index: number): Promise<never>;
    frames(): Promise<string>;
    switchToFrame(frameRef: string): Promise<void>;
    switchToMainFrame(): Promise<void>;
    click(selector: string): Promise<void>;
    type(selector: string, text: string): Promise<void>;
    fill(selector: string, value: string): Promise<void>;
    innerHTML(selector: string): Promise<string>;
    innerText(selector: string): Promise<string>;
    textContent(selector: string): Promise<string>;
    getAttribute(selector: string, name: string): Promise<string>;
    inputValue(selector: string): Promise<string>;
    isVisible(selector: string): Promise<boolean>;
    isEnabled(selector: string): Promise<boolean>;
    evaluate(expression: string): Promise<unknown>;
    frameEvaluate(frameRef: string, expression: string): Promise<unknown>;
    frameFill(frameRef: string, selector: string, value: string): Promise<void>;
    snapshot(options?: PageSnapshotOptions): Promise<string>;
    setDialogHandler(
        mode: 'accept' | 'dismiss' | 'none',
        promptText?: string,
    ): Promise<void>;
    lastDialog(): Promise<string>;
    setPopupHandler(mode: 'ignore' | 'same_tab'): Promise<void>;
    popupEvents(): Promise<string>;
    screenshot(): Promise<string>;
    waitForDownload(timeoutMs?: number): Promise<PageDownload>;
}

interface BrowserApi {
    pages(): Promise<PageApi[]>;
    waitForEvent(event: 'page', timeoutMs?: number): Promise<PageApi>;
}

interface SaveResourceOptions {
    coverageEndDate?: string | undefined;
    originalUrl?: string;
    mimeType?: string;
    label?: string;
    [key: string]: string | number | boolean | null | undefined;
}

interface SessionMetadata {
    dateRangeStart?: string;
    dateRangeEnd?: string;
}

interface RefreshmintApi {
    saveResource(
        filename: string,
        data: string | Uint8Array | number[] | ArrayLike<number>,
        options?: SaveResourceOptions,
    ): Promise<void>;
    saveDownloadedResource(
        path: string,
        filename?: string,
        options?: SaveResourceOptions,
    ): Promise<void>;
    listAccountDocuments(
        filter?:
            | string
            | {
                  label?: string;
                  [key: string]: string | number | boolean | null | undefined;
              },
    ): Promise<string>;
    setSessionMetadata(metadata: SessionMetadata): Promise<void>;
    reportValue(key: string, value: string): void;
    log(message: string): void;
    prompt(message: string): Promise<string>;
}

declare const page: PageApi;
declare const browser: BrowserApi;
declare const refreshmint: RefreshmintApi;
