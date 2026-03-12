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

type ScreenshotClip = {
    x: number;
    y: number;
    width: number;
    height: number;
};

type ScreenshotOptions = {
    type?: 'png' | 'jpeg';
    quality?: number;
    fullPage?: boolean;
    clip?: ScreenshotClip;
    omitBackground?: boolean;
    caret?: 'hide' | 'initial';
    animations?: 'disabled' | 'allow';
    scale?: 'css' | 'device';
    mask?: Locator[];
    maskColor?: string;
    style?: string;
    path?: string;
};

type RequestUrlMatcher =
    | string
    | RegExp
    | ((request: Request) => boolean | Promise<boolean>);
type ResponseUrlMatcher =
    | string
    | RegExp
    | ((response: Response) => boolean | Promise<boolean>);
type PageEventValueMap = {
    popup: PageApi;
    request: Request;
    response: Response;
    requestfinished: Request;
    requestfailed: Request;
};
type WaitForEventPredicate<E extends keyof PageEventValueMap> = (
    value: PageEventValueMap[E],
) => boolean | Promise<boolean>;

type ResourceTiming = {
    startTime: number;
    domainLookupStart: number;
    domainLookupEnd: number;
    connectStart: number;
    secureConnectionStart: number;
    connectEnd: number;
    requestStart: number;
    responseStart: number;
    responseEnd: number;
};

type RemoteAddr = {
    ipAddress: string;
    port: number;
};

type SecurityDetails = {
    protocol?: string;
    subjectName?: string;
    issuer?: string;
    validFrom?: number;
    validTo?: number;
};
type WaitForEventOptions<E extends keyof PageEventValueMap> = {
    timeout?: number;
    predicate?: WaitForEventPredicate<E>;
};

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
    screenshot(
        options?: Omit<ScreenshotOptions, 'fullPage' | 'clip'>,
    ): Promise<Uint8Array>;
    wait_for(options?: {
        state?: 'attached' | 'detached' | 'visible' | 'hidden';
        timeout?: number;
    }): Promise<void>;
}

interface JSHandle {
    dispose(): Promise<void>;
    jsonValue(): Promise<string>;
    toString(): string;
}

interface ElementHandle extends JSHandle {
    click(): Promise<void>;
    fill(value: string): Promise<void>;
    textContent(): Promise<string | null>;
    innerText(): Promise<string | null>;
    getAttribute(name: string): Promise<string | null>;
    isVisible(): Promise<boolean>;
    screenshot(
        options?: Omit<ScreenshotOptions, 'fullPage' | 'clip'>,
    ): Promise<Uint8Array>;
    $(selector: string): Promise<ElementHandle | null>;
    $$(selector: string): Promise<ElementHandle[]>;
}

interface Frame {
    url(): Promise<string>;
    name(): Promise<string>;
    parentFrame(): Promise<Frame | null>;
    page(): PageApi;
}

interface Request {
    url(): string;
    method(): string;
    resourceType(): string;
    headers(): Record<string, string>;
    allHeaders(): Promise<Record<string, string>>;
    headersArray(): Promise<Array<{ name: string; value: string }>>;
    headerValue(name: string): Promise<string | null>;
    isNavigationRequest(): boolean;
    postData(): Promise<string | null>;
    postDataBuffer(): Promise<Uint8Array | null>;
    postDataJSON(): Promise<unknown>;
    failure(): Promise<Record<string, unknown> | null>;
    response(): Promise<Response | null>;
    timing(): ResourceTiming;
    frame(): Frame | null;
    redirectedFrom(): Promise<Request | null>;
    redirectedTo(): Promise<Request | null>;
}

interface Response {
    url(): string;
    status(): number;
    ok(): boolean;
    statusText(): string;
    headers(): Record<string, string>;
    allHeaders(): Promise<Record<string, string>>;
    headersArray(): Promise<Array<{ name: string; value: string }>>;
    headerValue(name: string): Promise<string | null>;
    headerValues(name: string): Promise<string[]>;
    body(): Promise<Uint8Array>;
    text(): Promise<string>;
    json(): Promise<unknown>;
    request(): Promise<Request | null>;
    frame(): Frame | null;
    finished(): Promise<Record<string, unknown> | null>;
    fromServiceWorker(): boolean;
    serverAddr(): Promise<RemoteAddr | null>;
    securityDetails(): Promise<SecurityDetails | null>;
}

interface PageApi {
    locator(selector: string): Locator;
    getByRole(role: string, options?: ByRoleOptions): Locator;
    goto(
        url: string,
        options?: {
            waitUntil?: 'load' | 'domcontentloaded' | 'networkidle' | 'commit';
            timeout?: number;
        },
    ): Promise<void>;
    url(): Promise<string>;
    reload(): Promise<void>;
    waitForSelector(selector: string, timeoutMs?: number): Promise<void>;
    waitForNavigation(timeoutMs?: number): Promise<void>;
    waitForURL(pattern: string, timeoutMs?: number): Promise<void>;
    waitForLoadState(
        state?: 'load' | 'domcontentloaded' | 'networkidle' | 'commit',
        timeoutMs?: number,
    ): Promise<void>;
    waitForResponse(
        urlOrPredicate: ResponseUrlMatcher,
        options?: { timeout?: number } | number,
    ): Promise<Response>;
    /** Waits for a response matching urlPattern and returns its decoded body string.
     *  Works for cross-origin OOP iframes (uses CDP Network.getResponseBody). */
    waitForResponseBody(
        urlPattern: string,
        timeoutMs?: number,
    ): Promise<string>;
    waitForRequest(
        urlOrPredicate: RequestUrlMatcher,
        options?: { timeout?: number } | number,
    ): Promise<Request>;
    networkRequests(): Promise<string>;
    responsesReceived(): Promise<string>;
    clearNetworkRequests(): Promise<void>;
    waitForPopup(timeoutMs?: number): Promise<PageApi>;
    waitForEvent<E extends keyof PageEventValueMap>(
        event: E,
        optionsOrPredicate?:
            | number
            | WaitForEventPredicate<E>
            | WaitForEventOptions<E>,
    ): Promise<PageEventValueMap[E]>;
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
    evaluateHandle(expression: string): Promise<unknown>;
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
    screenshot(options?: ScreenshotOptions): Promise<Uint8Array>;
    waitForDownload(timeoutMs?: number): Promise<PageDownload>;
}

interface BrowserApi {
    pages(): Promise<PageApi[]>;
    waitForEvent(
        event: 'page',
        optionsOrPredicate?:
            | number
            | ((page: PageApi) => boolean | Promise<boolean>)
            | {
                  timeout?: number;
                  predicate?: (page: PageApi) => boolean | Promise<boolean>;
              },
    ): Promise<PageApi>;
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
    /** Returns CLI --option key/value pairs as a JS object. Returns {} when no options are supplied. */
    getOptions(): Record<string, unknown>;
}

declare const page: PageApi;
declare const browser: BrowserApi;
declare const refreshmint: RefreshmintApi;

declare module 'refreshmint:util' {
    interface InspectOptions {
        depth?: number;
    }

    export function inspect(value: unknown, options?: InspectOptions): string;
}
