# Playwright API + implementation sketch

## Scope

This repo is the full Playwright monorepo. For feasibility, this document covers the **exported JavaScript/TypeScript API surfaces** and their implementation shape:

- `playwright-core` client API (`packages/playwright-core/src/client/*` + in/out-of-process bootstraps)
- `playwright` / `@playwright/test` API (`packages/playwright/src/*` exports)
- CLI/driver exported functions
- component testing packages (`packages/playwright-ct-*`, `playwright-ct-core`)
- thin browser client (`packages/playwright-client`)

It does **not** enumerate every private/internal helper in the monorepo.

## 1) Entry points and what they expose

### `playwright` and browser-specific packages

- `packages/playwright/index.js`: re-exports `playwright-core`.
- `packages/playwright/index.mjs`: `export * from 'playwright-core'`, default export is `playwright-core`.
- `packages/playwright-chromium/index.js`, `playwright-firefox/index.js`, `playwright-webkit/index.js`: also re-export `playwright-core`.

### `playwright-core`

- `packages/playwright-core/index.js`: Node version guard, then `require('./lib/inprocess')`.
- `packages/playwright-core/index.mjs`: re-exports selected top-level fields from default Playwright object: `chromium`, `firefox`, `webkit`, `selectors`, `devices`, `errors`, `request`, `_electron`, `_android`.

### `@playwright/test`

- `packages/playwright-test/index.js` and `.mjs`: re-export `playwright/test`.
- `packages/playwright/src/index.ts`: defines `test`, re-exports `expect`, `defineConfig`, `mergeTests`, `mergeExpects`.

### component testing packages

- `playwright-ct-core/index.js`: exports `{ test, expect, devices, defineConfig }` built on top of `playwright/test` with CT plugin + transform wiring.
- `playwright-ct-react|vue|svelte/index.js`: wrappers around ct-core `defineConfig` that inject framework plugin factory and register source.

### thin browser client

- `packages/playwright-client/index.js` -> `./lib/index`
- `packages/playwright-client/index.mjs` exports `connect`.

## 2) Core architecture (how API calls are implemented)

1. Client classes (e.g. `Browser`, `Page`, `Locator`) extend `ChannelOwner`.
2. `ChannelOwner` creates a proxy `_channel`; method calls are validated and sent over `Connection.sendMessageToServer`.
3. `Connection` tracks objects by `guid`, dispatches `__create__/__dispose__/events`, and validates params/results via protocol validators.
4. Most API methods are **thin wrappers** that:
    - normalize options
    - compute timeout via `TimeoutSettings`
    - serialize arguments/headers/body
    - call channel method
    - rehydrate channel objects into typed wrappers
5. Event waits use `Waiter`, which adds timeout/cancellation/error wiring and appends call logs.

## 3) `playwright-core` public object + class APIs

## 3.1 Top-level Playwright object

`Playwright` (`client/playwright.ts`)

- Fields: `chromium`, `firefox`, `webkit`, `_android`, `_electron`, `devices`, `selectors`, `request`, `errors.TimeoutError`.
- Implementation: constructor rehydrates channels, wires browser types back to this Playwright instance, and initializes selector/request helpers.

## 3.2 Browser launch/connect

### `BrowserType`

- `executablePath()`: returns initializer path or throws unsupported-platform error.
- `name()`: returns browser type name.
- `launch(options)`: validates options, merges default launch opts, normalizes env/ignore args/timeout, channel `launch`, then binds browser to this type.
- `launchServer(options)`: delegates to injected server launcher (`BrowserServerLauncherImpl`).
- `launchPersistentContext(userDataDir, options)`: merges selector/default options, runs instrumentation hooks, builds context params (`prepareBrowserContextParams`), channel launch, initializes HAR, returns context.
- `connect(...)`: overload wrapper; normalizes args and calls `_connect`.
- `_connect(params)`: opens remote transport (`connectOverWebSocket`), initializes remote Playwright, validates prelaunched browser exists, wires disconnect handling and timeout logic.
- `connectOverCDP(...)`: overload wrapper for endpoint URL.
- `_connectOverCDP(endpointURL, params)`: Chromium-only channel call, converts headers, wires browser and default context instrumentation.

### `Browser`

- `newContext()` / `_newContextForReuse()`: delegates to `_innerNewContext` with reuse flag.
- `_innerNewContext(...)`: applies selector options + instrumentation hooks, builds context params, channel call, initializes HAR and context defaults.
- `newPage()`: creates owned context then page.
- `newBrowserCDPSession()`, `startTracing()`, `stopTracing()`: channel wrappers + artifact/file persistence.
- `close()`: closes via channel or underlying connection; swallows already-closed errors.
- Lifecycle tracking: `_didCreateContext`, `_setupBrowserContext`, `_didClose`, `isConnected()`, `contexts()`, `version()`.

### `BrowserContext`

- State/config: `setDefaultTimeout`, `setDefaultNavigationTimeout`, `browser`, `pages`.
- Page/context ops: `newPage`, `storageState`, `setStorageState`, `newCDPSession`, `close`.
- Cookies/permissions/network emulation: `cookies`, `addCookies`, `clearCookies`, `grantPermissions`, `clearPermissions`, `setGeolocation`, `setExtraHTTPHeaders`, `setOffline`, `setHTTPCredentials`.
- Script/bindings: `addInitScript`, `exposeBinding`, `exposeFunction`.
- Routing: `route`, `routeWebSocket`, `routeFromHAR`, `unroute`, `unrouteAll`.
- Event waiting: `waitForEvent` uses `Waiter` with close/crash/timeouts.
- Internal event bridge: `_onRoute`, `_onWebSocketRoute`, `_onBinding`, request/response propagation to page/context events.
- Safety gates: `_checkUrlAllowed` and `_checkFileAccess` enforce protocol/dir allowlists when configured.

### helpers used by Browser/BrowserContext

- `prepareBrowserContextParams(platform, options)`: validates/normalizes context options (headers/storageState/video/client certs/accept downloads/etc.) into protocol params.
- `toClientCertificatesProtocol(platform, certs)`: resolves cert buffers from inline values or file paths.

## 3.3 Page/frame/locator DOM APIs

### `Page`

Implementation shape:

- holds main frame + child frame set
- many selector/action methods simply delegate to `mainFrame` equivalents
- routing/event APIs are page-level wrappers over route handlers + channel events

Method groups:

- Frame-delegating convenience methods: `$`, `$$`, `$eval`, `$$eval`, `waitForSelector`, `click`, `dblclick`, `fill`, `type`, `press`, `focus`, `hover`, `textContent`, `innerText`, `innerHTML`, `getAttribute`, `inputValue`, `is*`, `selectOption`, `setInputFiles`, `waitForTimeout`, `title`, etc.
- Navigation/waiting: `goto`, `reload`, `goBack`, `goForward`, `waitForNavigation`, `waitForURL`, `waitForLoadState`, `waitForRequest`, `waitForResponse`, `waitForEvent`.
- Routing/interception: `route`, `routeWebSocket`, `routeFromHAR`, `unroute`, `unrouteAll`.
- Media/artifacts: `screenshot`, `_expectScreenshot`, `pdf`, `video`.
- Lifecycle: `close`, `isClosed`, `opener`, frame attach/detach/crash/close handling.
- Agent hooks: `agent()`, `_snapshotForAI()` call dedicated channel methods.

### `Frame`

- Core nav methods (`goto`, `waitForNavigation`, `waitForLoadState`, `waitForURL`) use `Waiter` and load-state validation.
- DOM/query/eval methods call frame channel methods and serialize args with `serializeArgument`.
- Selector/action methods are mostly direct channel wrappers with strict/timeout normalization.
- Locator constructors (`locator`, `getBy*`, `frameLocator`) are pure selector-string builders around frame context.

### `Locator` / `FrameLocator`

- `Locator` stores `{ frame, selector }` and composes selector DSL (`has`, `hasText`, visibility, `nth`, `and`, `or`, `describe`).
- High-level actions generally forward to corresponding frame methods with `strict: true`.
- `_withElement(...)` resolves element via `waitForSelector(attached)` and auto-disposes handle.
- `FrameLocator` composes frame-entry selectors and yields nested locators.
- `testIdAttributeName()` / `setTestIdAttribute()` manage global test-id attribute used by `getByTestId` selector generation.

### `ElementHandle`

- Channel-backed element operations: state checks, actions, eval/query methods, screenshot.
- `setInputFiles` uses `convertInputFiles` to support local files, buffers, remote transfer streams, and directory upload constraints.
- `screenshot` infers type via `determineScreenshotType`, maps mask locators to frame+selector protocol objects, optionally writes file.

### `JSHandle`

- eval/evalHandle/property/jsonValue wrappers over JS handle channel.
- `dispose()` suppresses target-closed errors.
- helpers:
    - `serializeArgument`: packs JSHandles into `handles` array + serialized payload.
    - `parseResult`: deserializes protocol values.
    - `assertMaxArguments`: enforces 1-arg payload policy.

## 3.4 Network/request APIs

### `APIRequest` / `APIRequestContext` / `APIResponse`

- `APIRequest.newContext`: applies instrumentation + inherited defaults, normalizes headers/storage/client certs, creates context channel.
- `APIRequestContext` HTTP verbs (`get/post/put/patch/delete/head`) forward to `fetch`.
- `_innerFetch` handles URL/request overloads, validates mutually-exclusive body options (`data/form/multipart`), encodes params/body, then channel `fetch`.
- `storageState({path})` optionally persists JSON to disk.
- `APIResponse.body/text/json/dispose`: fetches response body by `fetchUid`, with disposal checks.

### `Request` / `Route` / `Response` / `WebSocket` / `WebSocketRoute`

- `Request`: exposes method/url/headers/postData/timing/redirect chain, with lazy raw-header fetch.
- `Route`: single-consumption route handler with `continue/fallback/abort/fulfill/fetch`; tracks handled state and races calls with target-close scope.
- `RouteHandler` + `WebSocketRouteHandler`: URL-pattern matching, expiration (`times`), in-flight handler bookkeeping, optional error suppression behavior on unroute.
- `Response`: status/body/header APIs, plus `finished()` linked to request lifecycle.
- `WebSocket`: event bridge and `waitForEvent` with `Waiter` error/close guards.
- `WebSocketRoute`: bidirectional page/server socket proxy with optional custom handlers.
- `validateHeaders(headers)`: header value type enforcement.

## 3.5 Other client objects

- `Selectors`: registers custom selector engines (via injected script source), pushes selector config to existing contexts, manages test-id attribute propagation.
- `Keyboard`/`Mouse`/`Touchscreen`: thin channel wrappers for input actions.
- `Tracing`: starts/stops trace chunks, tracks stack collection via local utils, writes/assembles zip differently for local vs remote connections.
- `Coverage`: start/stop JS/CSS coverage channel wrappers.
- `Electron` / `ElectronApplication`: launch Electron and manage windows/context/event waits.
- `Android` / `AndroidDevice` / `AndroidInput` / `AndroidWebView`: channel wrappers for device discovery, gestures, shell/socket/file push/install, and webview->page bridge.

## 3.6 Core helper exports (free functions)

- `createInProcessPlaywright()`: creates local dispatcher + client connection pair, initializes Playwright channel object, injects browser/android server launchers.
- `start(env)` (out-of-process): forks Playwright driver process, binds pipe transport to client `Connection`, returns `{ playwright, stop }`.
- `printApiJson()`: prints generated `api.json`.
- `runDriver()`: starts stdio JSON-RPC bridge (`DispatcherConnection` + `PipeTransport`).
- `runServer(options)`: starts `PlaywrightServer` websocket endpoint.
- `launchBrowserServer(browserName, configFile?)`: loads launch-server options and prints ws endpoint.
- `connectOverWebSocket(parentConnection, params)`: creates remote `Connection` over JSON pipe or browser WebSocket transport, with close/error forwarding.
- `envObjectToArray`, `evaluationScript`, `addSourceUrlToScript`: utility conversions for env/script injection.
- `isTargetClosedError`, `serializeError`, `parseError`: error classification/serialization helpers.
- `convertSelectOptionValues`, `convertInputFiles`, `determineScreenshotType`: select/file/screenshot option normalization helpers.
- `prepareBrowserContextParams`, `toClientCertificatesProtocol`, `verifyLoadState`, `mkdirIfNeeded`, `createInstrumentation`, `captureLibraryStackTrace`.

## 4) `@playwright/test` API and implementation

Top-level exports from `packages/playwright/src/index.ts`:

- `expect`
- `_baseTest`
- `test` (extended with Playwright fixtures)
- `defineConfig` (re-export)
- `mergeTests` (re-export)
- `mergeExpects` (re-export)

### `test` object (`TestTypeImpl`)

`TestTypeImpl` builds the function object with attached DSL:

- creators: `test()`, `test.only`, `test.fail.only`
- suite builders: `test.describe`, `.only`, `.parallel`, `.serial`, `.skip`, `.fixme`, `.configure`
- hooks: `beforeEach`, `afterEach`, `beforeAll`, `afterAll`
- modifiers: `skip`, `fixme`, `fail`, `slow`, `setTimeout`
- steps: `test.step`, `test.step.skip`
- fixture composition: `test.use`, `test.extend`
- runtime info: `test.info()`

Implementation sketch:

- while loading test files, APIs mutate a tree of `Suite`/`TestCase` objects (`currentlyLoadingFileSuite`).
- validations are done eagerly (details/tags, nesting/parallel-mode constraints, illegal usage context).
- step execution uses zone context + deadline racing, and reports step completion/failure to test info.

### `defineConfig` (`common/configLoader.ts`)

- Accepts one or more config objects and deep-merges key sections (`expect`, `use`, `build`, `webServer`, project overrides by name).
- Marks result with internal symbol so runtime can detect `defineConfig` usage.

### `mergeTests` (`common/testType.ts`)

- Accepts multiple `test` objects (must carry internal test-type symbol), merges fixture layers while deduping shared fixture ancestors.

### `expect` / `mergeExpects` (`matchers/expect.ts`)

- `expect` is a proxied wrapper around expect library extended with Playwright async matchers.
- Proxy tracks metadata (`soft`, `not`, `poll`, timeout, message), creates reporter steps, and maps matcher failures to `ExpectError` with filtered stacks.
- `mergeExpects(...expects)`: merges custom matcher registries from playwright expects.

### `FullConfigInternal` / `FullProjectInternal` (`common/config.ts`)

- Build normalized runtime config from user config + CLI overrides.
- Resolve scripts/reporters/projects/dependencies, defaults, workers, tags, output dirs, etc.
- helper exports: `takeFirst`, `toReporters`, `getProjectId`.

## 5) Runner + CLI exported functions

### runner exports

- `runAllTestsWithConfig(config)`: prepares plugins/reporters/tasks and executes test pipeline.
- `runTasks`, `runTasksDeferCleanup`: task-runner orchestration with global timeout/deadline and reporter finalization.
- task builders: `createGlobalSetupTasks`, `createRunTestsTasks`, `createClearCacheTask`, `createReportBeginTask`, `createPluginSetupTasks`, `createListFilesTask`, `createLoadTask`, `createApplyRebaselinesTask`, `createStartDevServerTask`.
- reporter factory exports: `createReporters`, `createReporterForTestServer`, `createErrorCollectingReporter`.
- project utilities: `filterProjects`, `buildTeardownToSetupsMap`, `buildProjectsClosure`, `findTopLevelProjects`, `buildDependentProjects`, `collectFilesForProject`.
- loader utilities: `collectProjectsAndTestFiles`, `loadFileSuites`, `createRootSuite`, `loadGlobalHook`, `loadReporter`, `loadTestList`.

Implementation pattern:

- all of these build/transform execution graphs (`Suite`, project closures, phases, task queue), then drive workers/dispatchers via `TaskRunner`.

### CLI (`packages/playwright/src/program.ts`)

- Exports `program` (from `playwright-core` CLI parser).
- Adds commands (`test`, `show-report`, `merge-reports`, `clear-cache`, test server/dev server, MCP commands, `init-agents`).
- `runTests` path: parse CLI overrides, load config, branch into UI/watch/standard run, exit with mapped status code.

### driver CLI (`playwright-core/src/cli/driver.ts`)

- `printApiJson`, `runDriver`, `runServer`, `launchBrowserServer` as described above.

## 6) Component-testing API (`playwright-ct-core`)

### ct-core runtime exports

- `fixtures` (`mount.ts`): provides `mount` and `router` fixtures and enforces CT preconditions.
- `runDevServer(config)`: starts Vite dev server, injects CT transform plugin, watches test dirs for component import registry changes.
- `createPlugin()`: test-runner plugin implementation for bundle/dev server lifecycle and dependency population.
- `buildBundle(config, configDir)`: incremental build metadata check + Vite build + dependency extraction.
- vite utils:
    - `resolveDirs`, `resolveEndpoint`, `createConfig`, `populateComponentsFromTests`, `hasJSComponents`, `transformIndexFile`, `frameworkConfig`.
- transform util:
    - `importInfo(...)`: turns component imports into stable import-ref IDs for CT runtime registry.

Implementation pattern:

- CT builds a component registry from transformed tests, injects lazy dynamic imports into template `index.*`, then serves bundle via Vite preview/dev server.

## 7) Thin browser client API (`playwright-client`)

- `connect(wsEndpoint, browserName, options)`:
    - opens browser WebSocket endpoint
    - wires socket <-> `Connection` JSON dispatch
    - initializes remote Playwright
    - returns prelaunched browser (`playwright._preLaunchedBrowser()`).

## 8) “How each function is implemented” quick taxonomy

Across all exported functions in this scan, implementation falls into these buckets:

- **RPC wrapper**: normalize args -> call `_channel.method` -> map protocol result to object (`from(...)`) or primitive.
- **Config merge/normalize**: deterministic object merge with CLI/user precedence and type validation.
- **Orchestration**: build tasks/phases/reporters and execute with task runner and failure tracking.
- **Selector/string composition**: build internal selector DSL strings (`Locator`, `FrameLocator`, `getBy*`).
- **Resource/IO helpers**: read/write files, zip traces, upload streams, convert headers/body formats.
- **Event waiter**: subscribe + timeout + cancellation guards + enriched error logs (`Waiter`).

## 9) Chromium CDP RPC sketch for each `playwright-core` method in this doc

Scope note: this section is specifically the **Chromium/CDP backend** (`server/chromium/*`). For Firefox/WebKit, transport and protocol differ.

Legend:

- `A -> B` means public method `A` delegates to public/internal method `B`.
- `CDP:` lists concrete DevTools protocol commands used in Chromium implementation.

### 9.1 `BrowserType`

- `BrowserType.launch()`
    - `launch()` -> channel `BrowserType.launch` -> Chromium launch flow (`Chromium.launch` / `CRBrowser.connect`).
    - CDP bootstrap includes: `Browser.getVersion`, `Target.setAutoAttach`, `Target.getTargetInfo` (persistent context stabilization).
- `BrowserType.launchPersistentContext()`
    - -> channel `launchPersistentContext` -> `Target.createBrowserContext` + context init.
    - CDP: `Target.createBrowserContext`, then context/page init sequence (see `BrowserContext.newPage` and `FrameSession._initialize`).
- `BrowserType.connect()`
    - uses websocket transport + Playwright protocol; no single direct CDP command at this API boundary.
- `BrowserType.connectOverCDP()`
    - -> Chromium `_connectOverCDPInternal` (attach to existing CDP endpoint).
    - CDP after connect: `Browser.getVersion`, `Target.setAutoAttach`, optional `Target.getTargetInfo`.

### 9.2 `Browser`

- `Browser.newContext()`
    - -> `CRBrowser.doCreateNewContext`.
    - CDP: `Target.createBrowserContext`; context init includes `Browser.setDownloadBehavior`.
- `Browser.newPage()`
    - `newPage()` -> `newContext()` + `context.newPage()`.
    - CDP: `Target.createTarget` (`about:blank`) in `CRBrowserContext.doCreateNewPage`.
- `Browser.newBrowserCDPSession()`
    - -> `CRBrowser.newBrowserCDPSession`.
    - CDP: `Target.attachToBrowserTarget`.
- `Browser.startTracing()` / `Browser.stopTracing()`
    - -> `CRBrowser.startTracing` / `stopTracing`.
    - CDP: `Tracing.start`, `Tracing.end`, then protocol stream read via `IO.read` + `IO.close`.
- `Browser.close()`
    - closes channel/transport.
    - CDP close path (Chromium graceful): `Browser.close` (sent as protocol message id `kBrowserCloseMessageId`) or `Target.closeTarget` for page close.

### 9.3 `BrowserContext`

- `cookies()`
    - -> `CRBrowserContext.doGetCookies`.
    - CDP: `Storage.getCookies`.
- `addCookies()`
    - -> `CRBrowserContext.addCookies`.
    - CDP: `Storage.setCookies`.
- `clearCookies()`
    - -> `CRBrowserContext.doClearCookies`.
    - CDP: `Storage.clearCookies`.
- `grantPermissions()` / `clearPermissions()`
    - -> `CRBrowserContext.doGrantPermissions` / `doClearPermissions`.
    - CDP: `Browser.grantPermissions`, `Browser.resetPermissions`.
- `setGeolocation()`
    - -> `CRBrowserContext.setGeolocation` -> `CRPage.updateGeolocation` -> `FrameSession._updateGeolocation`.
    - CDP: `Emulation.setGeolocationOverride`.
- `setExtraHTTPHeaders()`
    - -> `CRPage.updateExtraHTTPHeaders` / `CRServiceWorker.updateExtraHTTPHeaders` -> network manager.
    - CDP: `Network.setExtraHTTPHeaders`.
- `setOffline()`
    - -> `CRPage.updateOffline` / SW update -> `CRNetworkManager.setOffline`.
    - CDP: `Network.emulateNetworkConditions`.
- `setHTTPCredentials()`
    - -> `CRNetworkManager.authenticate` + interception update.
    - CDP: enables fetch interception (`Fetch.enable`), handles auth via `Fetch.continueWithAuth` on `Fetch.authRequired`.
- `addInitScript()`
    - -> `CRPage.addInitScript` -> `FrameSession._evaluateOnNewDocument`.
    - CDP: `Page.addScriptToEvaluateOnNewDocument`.
- `exposeBinding()` / `exposeFunction()`
    - -> binding install path in frame session.
    - CDP: `Runtime.addBinding` + init script injection (`Page.addScriptToEvaluateOnNewDocument`) for glue code.
- `route()` / `routeFromHAR()` / `unroute()`
    - routing handled in client and network manager.
    - CDP interception core: `Fetch.enable`, `Fetch.continueRequest`, `Fetch.fulfillRequest`, `Fetch.failRequest`, plus request events from `Network.*`.
- `newPage()`
    - -> `CRBrowserContext.doCreateNewPage`.
    - CDP: `Target.createTarget`.
- `newCDPSession(pageOrFrame)`
    - -> `CRBrowserContext.newCDPSession`.
    - CDP: `Target.attachToTarget`.
- `close()`
    - -> `CRBrowserContext.doClose`.
    - CDP: `Target.disposeBrowserContext` (or `Browser.close` for persistent default context).

### 9.4 `Page`

- `bringToFront()`
    - CDP: `Page.bringToFront`.
- `reload()`
    - CDP: `Page.reload`.
- `goBack()` / `goForward()`
    - CDP: `Page.getNavigationHistory` + `Page.navigateToHistoryEntry`.
- `requestGC()`
    - CDP: `HeapProfiler.collectGarbage`.
- `close({ runBeforeUnload })`
    - `runBeforeUnload: true` -> `Page.close`.
    - otherwise target close via `Target.closeTarget`.
- `screenshot()`
    - -> page delegate screenshot path.
    - CDP: `Page.getLayoutMetrics` + `Page.captureScreenshot`.
- `pdf()`
    - -> `CRPDF.generate`.
    - CDP: `Page.printToPDF` + stream `IO.read`/`IO.close`.
- `setViewportSize()` (through context/page emulation state)
    - -> `FrameSession._updateViewport`.
    - CDP: `Emulation.setDeviceMetricsOverride`, and in headed mode `Browser.getWindowBounds` / `Browser.setWindowBounds`.
- `emulateMedia()`
    - -> `FrameSession._updateEmulateMedia`.
    - CDP: `Emulation.setEmulatedMedia`.
- `route(...)` family
    - same interception flow as context routes (`Fetch.*`, `Network.*`).
- `waitForRequest()`
    - `waitForRequest` -> `waitForEvent(Page.Request)`.
    - Event source in Chromium: `Network.requestWillBeSent` (+ extra info events).
- `waitForResponse()`
    - `waitForResponse` -> `waitForEvent(Page.Response)`.
    - Event source in Chromium: `Network.responseReceived` (+ `Network.responseReceivedExtraInfo` alignment).
- `waitForNavigation()` / `waitForLoadState()` / `waitForURL()`
    - uses `Waiter` over frame/page events.
    - Event source in Chromium: `Page.frameNavigated`, `Page.navigatedWithinDocument`, `Page.lifecycleEvent` (`load`, `DOMContentLoaded`).

### 9.5 `Frame`

- `goto(url)`
    - -> delegate `CRPage.navigateFrame` -> `FrameSession._navigate`.
    - CDP: `Page.navigate`.
- `waitForNavigation()` / `waitForLoadState()` / `waitForURL()`
    - no direct RPC; waits on navigation/lifecycle events produced by:
    - CDP events: `Page.frameNavigated`, `Page.navigatedWithinDocument`, `Page.lifecycleEvent`.
- `evaluate()` / `evaluateHandle()`
    - through execution context delegate.
    - CDP: `Runtime.evaluate` / `Runtime.callFunctionOn`.
- selector/action APIs (`click`, `fill`, `type`, `press`, `hover`, etc.)
    - implemented via injected scripts + element actions.
    - CDP core primitives involved:
        - DOM/query/geometry: `DOM.describeNode`, `DOM.resolveNode`, `DOM.getBoxModel`, `DOM.getContentQuads`, `DOM.scrollIntoViewIfNeeded`
        - JS execution: `Runtime.callFunctionOn`, `Runtime.getProperties`
        - input synthesis: `Input.dispatchMouseEvent`, `Input.dispatchKeyEvent`, `Input.insertText`, `Input.dispatchTouchEvent`.

### 9.6 `Locator` and `FrameLocator`

- Most methods are wrappers over `Frame` methods with `strict: true`.
- CDP path therefore equals the corresponding `Frame`/`ElementHandle` path above.
- `count()` path uses frame query count pipeline (selector engine in injected script via runtime evaluation).
- `waitFor()` uses `Frame.waitForSelector` -> runtime polling/selectors + frame lifecycle timeouts.

### 9.7 `ElementHandle`

- `ownerFrame()` / `contentFrame()`
    - CDP: `DOM.describeNode`, `DOM.getFrameOwner` (through frame-session helpers).
- geometry/visibility helpers:
    - `boundingBox()` -> `DOM.getBoxModel`
    - `scrollIntoViewIfNeeded()` -> `DOM.scrollIntoViewIfNeeded`
    - content quads path uses `DOM.getContentQuads`.
- `setInputFiles()`
    - CDP: `DOM.setFileInputFiles`.
- `screenshot()`
    - delegates to page screenshot path -> `Page.captureScreenshot` (+ metrics call).
- eval/property methods
    - CDP: `Runtime.callFunctionOn`, `Runtime.getProperties`, `Runtime.releaseObject`.

### 9.8 `JSHandle`

- `evaluate()` / `evaluateHandle()`
    - CDP: `Runtime.callFunctionOn` (or `Runtime.evaluate` depending path).
- `getProperty()` / `getProperties()`
    - CDP: `Runtime.getProperties`.
- `dispose()`
    - CDP: `Runtime.releaseObject`.

### 9.9 `Request` / `Response` / `Route` / `WebSocket`

- `Request.*` getters
    - populated from CDP network events.
    - CDP sources: `Network.requestWillBeSent`, `Network.requestWillBeSentExtraInfo`.
- `Response.body()`
    - CDP: `Network.getResponseBody`; fallback path may use `Network.loadNetworkResource` + `IO.read` + `IO.close`.
- `Route.continue()`
    - CDP: `Fetch.continueRequest`.
- `Route.fulfill()`
    - CDP: `Fetch.fulfillRequest`.
- `Route.abort()`
    - CDP: `Fetch.failRequest`.
- `WebSocket` events
    - CDP event sources: `Network.webSocketCreated`, `Network.webSocketWillSendHandshakeRequest`, `Network.webSocketHandshakeResponseReceived`, `Network.webSocketFrameSent`, `Network.webSocketFrameReceived`, `Network.webSocketClosed`, `Network.webSocketFrameError`.

### 9.10 `APIRequest` / `APIRequestContext` / `APIResponse`

- Not backed by browser CDP network stack.
- Implemented through Playwright server-side HTTP client (`fetch` pipeline), then exposed through Playwright protocol channels.
- So this section maps these methods via internal HTTP logic, not CDP RPC.

### 9.11 `Keyboard` / `Mouse` / `Touchscreen`

- `Keyboard.down/up/press/type/insertText`
    - CDP: `Input.dispatchKeyEvent`, `Input.insertText`.
- `Mouse.move/down/up/click/dblclick/wheel`
    - CDP: `Input.dispatchMouseEvent`.
- `Touchscreen.tap`
    - CDP: `Input.dispatchTouchEvent` (`touchStart` then `touchEnd`).

### 9.12 `Tracing` and `Coverage`

- `Tracing.start()` / `startChunk()` / `stopChunk()` / `stop()` (browser-level tracing)
    - CDP: `Tracing.start`, `Tracing.end`; stream retrieval via `IO.read`/`IO.close`.
- JS coverage:
    - `startJSCoverage()` -> `Profiler.enable`, `Profiler.startPreciseCoverage`, `Debugger.enable`, `Debugger.setSkipAllPauses`.
    - `stopJSCoverage()` -> `Profiler.takePreciseCoverage`, `Profiler.stopPreciseCoverage`, `Profiler.disable`, `Debugger.disable`.
    - script text lookup: `Debugger.getScriptSource`.
- CSS coverage:
    - `startCSSCoverage()` -> `DOM.enable`, `CSS.enable`, `CSS.startRuleUsageTracking`.
    - `stopCSSCoverage()` -> `CSS.stopRuleUsageTracking`, then `CSS.disable`, `DOM.disable`.
    - stylesheet text lookup: `CSS.getStyleSheetText`.

### 9.13 `Selectors`

- `selectors.register()` / `setTestIdAttribute()` are primarily selector-engine bookkeeping and script injection wiring.
- They do not map to one fixed CDP command; selectors are executed through frame evaluation/runtime paths (`Runtime.callFunctionOn`) during actual locator/query/action operations.

### 9.14 `Electron` / `Android`

- These are not pure Chromium-page-CDP wrappers in the same shape as `Page/Frame`.
- They use Playwright protocol objects that internally may use multiple transports (including CDP for webview/page parts), so mapping is mixed and not a single CRPage/CDP path per method.

## 10) Example requested: `waitForResponse`

- Public API: `Page.waitForResponse(urlOrPredicate, options)`.
- Client implementation: wraps `waitForEvent(Page.Response, predicate)` with timeout/error guards.
- Event emission path:
    - CDP `Network.responseReceived` (+ `Network.responseReceivedExtraInfo`) in `CRNetworkManager`.
    - `CRNetworkManager._onResponseReceived` creates `network.Response` and notifies frame manager.
    - Frame manager emits Playwright `Page.Response` event.
- Result: `waitForResponse` itself is event-wait logic; the underlying CDP dependency is `Network.responseReceived` (with extra-info/header/body follow-up via `Network.*`/`IO.*` as needed).

## 11) Per-method CDP sketch (explicit map)

This section is intentionally explicit about delegation, so each public `playwright-core` method here is either:

- mapped to concrete CDP RPCs, or
- mapped to another public method that is mapped to CDP in this same section.

### 11.1 `BrowserType` methods

- `executablePath()`, `name()`:
    - local initializer reads; no CDP.
- `launch()`:
    - Playwright protocol `BrowserType.launch` -> Chromium launch/bootstrap.
    - CDP (bootstrap and target wiring): `Browser.getVersion`, `Target.setAutoAttach`, `Target.getTargetInfo`.
- `launchPersistentContext(userDataDir, options)`:
    - Playwright protocol `launchPersistentContext` -> browser/context creation.
    - CDP: `Target.createBrowserContext`, then context/page bootstrap (same primitives as `Browser.newContext()` + `BrowserContext.newPage()`).
- `connect(wsEndpoint, options)`:
    - Playwright protocol over WS to existing Playwright server; not a direct CDP API boundary.
- `connectOverCDP(endpointURL, options)`:
    - Playwright protocol call `connectOverCDP` -> Chromium CDP attach.
    - CDP attach/bootstrap: `Browser.getVersion`, `Target.setAutoAttach`, optional `Target.getTargetInfo`.

### 11.2 `Browser` methods

- `newContext(options)`:
    - `newContext()` -> `_innerNewContext()`.
    - CDP: `Target.createBrowserContext`, context init includes `Browser.setDownloadBehavior`.
- `_newContextForReuse(options)`:
    - same CDP path as `newContext()` (reuse semantics are Playwright-side policy).
- `newPage(options)`:
    - `newPage()` -> `newContext()` -> `BrowserContext.newPage()`.
    - CDP: `Target.createBrowserContext` then `Target.createTarget`.
- `newBrowserCDPSession()`:
    - CDP: `Target.attachToBrowserTarget`.
- `startTracing(page?, options)`:
    - CDP: `Tracing.start`.
- `stopTracing()`:
    - CDP: `Tracing.end` then stream pull via `IO.read` and `IO.close`.
- `close(options)`:
    - Playwright protocol close.
    - Chromium close paths use `Browser.close` (browser) and/or `Target.closeTarget` (target/page close).
- `contexts()`, `version()`, `isConnected()`, `browserType()`:
    - local object state; no direct CDP.

### 11.3 `BrowserContext` methods

- `newPage()`:
    - CDP: `Target.createTarget`.
- `cookies(urls?)`:
    - CDP: `Storage.getCookies`.
- `addCookies(cookies)`:
    - CDP: `Storage.setCookies`.
- `clearCookies(options?)`:
    - CDP: `Storage.clearCookies`.
- `grantPermissions(permissions, {origin})`:
    - CDP: `Browser.grantPermissions`.
- `clearPermissions()`:
    - CDP: `Browser.resetPermissions`.
- `setGeolocation(geolocation)`:
    - CDP: `Emulation.setGeolocationOverride`.
- `setExtraHTTPHeaders(headers)`:
    - CDP: `Network.setExtraHTTPHeaders`.
- `setOffline(offline)`:
    - CDP: `Network.emulateNetworkConditions`.
- `setHTTPCredentials(credentials)`:
    - interception/auth flow via Fetch domain.
    - CDP: `Fetch.enable` and `Fetch.continueWithAuth` on auth challenges.
- `addInitScript(script, arg?)`:
    - CDP: `Page.addScriptToEvaluateOnNewDocument` (per page/session).
- `exposeBinding(name, callback, options?)`, `exposeFunction(name, callback)`:
    - CDP: `Runtime.addBinding` + script glue via `Page.addScriptToEvaluateOnNewDocument`.
- `route(url, handler, options?)`, `unroute(...)`, `unrouteAll(...)`:
    - Playwright route tables -> network interception enable/disable.
    - CDP core: `Fetch.enable`, `Fetch.disable`, `Fetch.continueRequest`, `Fetch.fulfillRequest`, `Fetch.failRequest`.
- `routeWebSocket(url, handler)`:
    - event interception over Network websocket events (`Network.webSocket*`).
- `routeFromHAR(...)`:
    - primarily HAR router logic; when serving mocked entries it fulfills via same route interception path (`Fetch.fulfillRequest`).
- `waitForEvent(event, ...)`:
    - waiter over context events. Underlying event sources are CDP events (network/target/page lifecycle) depending on event.
- `storageState(...)`, `setStorageState(...)`:
    - mixed implementation (cookies/storage scripting + protocol). CDP usage includes storage/cookie primitives and page/runtime evaluation paths.
- `newCDPSession(pageOrFrame)`:
    - CDP: `Target.attachToTarget`.
- `close(options)`:
    - CDP: `Target.disposeBrowserContext` (or browser close for persistent default context).
- `setDefaultTimeout`, `setDefaultNavigationTimeout`, `browser`, `pages`, `backgroundPages`, `serviceWorkers`:
    - local state/event views.

### 11.4 `Page` methods (direct/non-main-frame wrappers)

- `reload(options)`:
    - CDP: `Page.reload`.
- `goBack(options)`, `goForward(options)`:
    - CDP: `Page.getNavigationHistory` + `Page.navigateToHistoryEntry`.
- `requestGC()`:
    - CDP: `HeapProfiler.collectGarbage`.
- `emulateMedia(options)`:
    - CDP: `Emulation.setEmulatedMedia`.
- `setViewportSize(viewport)`:
    - CDP: `Emulation.setDeviceMetricsOverride`; headed resizing path also uses `Browser.getWindowBounds` / `Browser.setWindowBounds`.
- `addInitScript(script, arg?)`:
    - CDP: `Page.addScriptToEvaluateOnNewDocument`.
- `exposeBinding(name, callback, options?)`, `exposeFunction(name, callback)`:
    - CDP: `Runtime.addBinding` + `Page.addScriptToEvaluateOnNewDocument`.
- `setExtraHTTPHeaders(headers)`:
    - CDP: `Network.setExtraHTTPHeaders`.
- `route(...)`, `unroute(...)`, `unrouteAll(...)`, `routeFromHAR(...)`:
    - same interception mechanics as `BrowserContext.route*` (Fetch domain commands).
- `routeWebSocket(...)`:
    - websocket interception/event layer over `Network.webSocket*` events.
- `screenshot(options)`:
    - CDP: `Page.getLayoutMetrics`, `Page.captureScreenshot`.
- `_expectScreenshot(options)`:
    - repeated screenshot/assert pipeline; screenshot primitive is `Page.captureScreenshot`.
- `pdf(options)`:
    - CDP: `Page.printToPDF`; stream read via `IO.read` + `IO.close` when returned as stream.
- `bringToFront()`:
    - CDP: `Page.bringToFront`.
- `close({ runBeforeUnload })`:
    - `runBeforeUnload: true` -> CDP `Page.close`.
    - default force-close path -> CDP `Target.closeTarget`.
- `consoleMessages()`:
    - events collected from runtime/logging channels (CDP includes `Runtime.consoleAPICalled` and `Log.entryAdded`).
- `pageErrors()`:
    - based on runtime exception events (CDP `Runtime.exceptionThrown`).
- `requests()`:
    - request list sourced from network events (`Network.requestWillBeSent`, related extra-info events).
- `waitForRequest(urlOrPredicate, options)`:
    - `waitForRequest` -> `_waitForEvent(Page.Request)`.
    - CDP event source: `Network.requestWillBeSent` (+ `Network.requestWillBeSentExtraInfo`).
- `waitForResponse(urlOrPredicate, options)`:
    - `waitForResponse` -> `_waitForEvent(Page.Response)`.
    - CDP event source: `Network.responseReceived` (+ `Network.responseReceivedExtraInfo`).
- `waitForEvent(event, ...)`:
    - generic waiter over already-emitted Playwright events; CDP source depends on event.
- `agent(...)`, `_snapshotForAI(...)`, `pause(...)`:
    - Playwright-specific higher-level channels; no single stable CDP mapping.
- `context()`, `opener()`, `mainFrame()`, `frame(...)`, `frames()`, `video()`, `isClosed()`, `workers()`, `url()`, `setDefaultTimeout(...)`, `setDefaultNavigationTimeout(...)`:
    - local object/event state accessors; no direct CDP.

### 11.5 `Page` methods that delegate to `Frame`

All of these are direct `page._mainFrame.<method>` forwarding. Use the `Frame` mapping in 11.6:

- Queries/eval: `$`, `$$`, `$eval`, `$$eval`, `evaluate`, `evaluateHandle`.
- DOM content/nav: `content`, `setContent`, `goto`, `title`, `waitForLoadState`, `waitForNavigation`, `waitForURL`, `waitForSelector`, `waitForFunction`, `waitForTimeout`.
- DOM actions: `dispatchEvent`, `click`, `dblclick`, `dragAndDrop`, `tap`, `fill`, `focus`, `hover`, `type`, `press`, `check`, `uncheck`, `setChecked`, `selectOption`, `setInputFiles`.
- DOM reads/state: `textContent`, `innerText`, `innerHTML`, `getAttribute`, `inputValue`, `isChecked`, `isDisabled`, `isEditable`, `isEnabled`, `isHidden`, `isVisible`.
- Script/style injection: `addScriptTag`, `addStyleTag`.
- Locator factories: `locator`, `getByTestId`, `getByAltText`, `getByLabel`, `getByPlaceholder`, `getByText`, `getByTitle`, `getByRole`, `frameLocator`.

### 11.6 `Frame` methods

- `goto(url, options)`:
    - CDP: `Page.navigate`.
- `waitForNavigation(options)`, `waitForLoadState(state, options)`, `waitForURL(url, options)`:
    - event waiters driven by CDP page/frame lifecycle events:
    - `Page.frameNavigated`, `Page.navigatedWithinDocument`, `Page.lifecycleEvent`.
- `frameElement()`:
    - frame-owner resolution path using DOM domain.
    - CDP: `DOM.getFrameOwner`, `DOM.resolveNode`.
- `evaluate(pageFunction, arg?)`, `evaluateHandle(...)`, `_evaluateFunction(...)`:
    - CDP: `Runtime.evaluate` and/or `Runtime.callFunctionOn`.
- `$(selector)`, `$$(selector)`, `$eval(...)`, `$$eval(...)`, `_queryCount(selector)`:
    - selector engine + eval path over runtime/DOM.
    - CDP primitives: `Runtime.callFunctionOn`, `Runtime.getProperties`, and node resolution via DOM commands as needed.
- `waitForSelector(selector, options)`:
    - polling/actionability in injected script + runtime.
    - CDP primitives: `Runtime.callFunctionOn` (+ DOM resolution).
- `dispatchEvent(selector, type, init, options)`:
    - runtime dispatch in page context.
    - CDP: `Runtime.callFunctionOn`.
- `content()`:
    - runtime evaluation of document content.
    - CDP: `Runtime.evaluate`.
- `setContent(html, options)`:
    - runtime DOM set + lifecycle wait.
    - CDP: runtime eval + lifecycle event stream (`Page.lifecycleEvent`).
- `addScriptTag(options)`, `addStyleTag(options)`:
    - DOM/script injection by runtime evaluation.
    - CDP: `Runtime.callFunctionOn`/`Runtime.evaluate`.
- `click`, `dblclick`, `tap`, `dragAndDrop`, `hover`:
    - actionability/geometry + input dispatch.
    - CDP core: `DOM.scrollIntoViewIfNeeded`, `DOM.getContentQuads`/`DOM.getBoxModel`, `Input.dispatchMouseEvent`, `Input.dispatchTouchEvent`, `Input.dispatchDragEvent`.
- `fill`, `type`, `press`, `focus`, `selectText`, `check`, `uncheck`, `setChecked`:
    - runtime focus/state manipulation + keyboard/input synthesis.
    - CDP core: `Runtime.callFunctionOn`, `Input.dispatchKeyEvent`, `Input.insertText`.
- `textContent`, `innerText`, `innerHTML`, `getAttribute`, `inputValue`, `isChecked`, `isDisabled`, `isEditable`, `isEnabled`, `isHidden`, `isVisible`, `title`:
    - runtime getter/evaluator path.
    - CDP: `Runtime.callFunctionOn` / `Runtime.evaluate`.
- `selectOption(...)`:
    - runtime option selection script.
    - CDP: `Runtime.callFunctionOn`.
- `setInputFiles(selector, files, options)`:
    - element upload primitive.
    - CDP: `DOM.setFileInputFiles`.
- `waitForFunction(pageFunction, arg, options)`:
    - repeated runtime evaluation until predicate passes.
    - CDP: `Runtime.callFunctionOn` / `Runtime.evaluate`.
- `waitForTimeout(timeout)`:
    - timer wait; no CDP.
- `locator(...)`, `getBy*`, `frameLocator(...)`:
    - selector construction only; CDP happens when an action/read method executes.

### 11.7 `Locator` / `FrameLocator` methods

- Core rule: most `Locator` actions and assertions call corresponding `Frame` methods with `{ strict: true }`.
- Therefore CDP mapping is exactly the mapping in 11.6 for the forwarded method (`click`, `fill`, `press`, `hover`, `waitFor`, etc.).
- Composition-only methods (`locator()`, `nth()`, `first()`, `last()`, `and()`, `or()`, `filter()`, `describe()`) are selector-string transforms and have no immediate CDP call.

### 11.8 `ElementHandle` methods

- `ownerFrame()`, `contentFrame()`:
    - frame/node resolution via DOM domain.
    - CDP: `DOM.describeNode`, `DOM.getFrameOwner`.
- `getAttribute`, `inputValue`, `textContent`, `innerText`, `innerHTML`, `isChecked`, `isDisabled`, `isEditable`, `isEnabled`, `isHidden`, `isVisible`:
    - CDP: `Runtime.callFunctionOn`.
- `dispatchEvent(...)`:
    - CDP: `Runtime.callFunctionOn`.
- `scrollIntoViewIfNeeded(...)`:
    - CDP: `DOM.scrollIntoViewIfNeeded`.
- `hover`, `click`, `dblclick`, `tap`:
    - CDP: geometry/actionability + `Input.dispatchMouseEvent` / `Input.dispatchTouchEvent`.
- `fill`, `selectText`, `focus`, `type`, `press`, `check`, `uncheck`, `setChecked`:
    - runtime + input dispatch.
    - CDP: `Runtime.callFunctionOn`, `Input.dispatchKeyEvent`, `Input.insertText`.
- `selectOption(...)`:
    - CDP: `Runtime.callFunctionOn`.
- `setInputFiles(...)`:
    - CDP: `DOM.setFileInputFiles`.
- `boundingBox()`:
    - CDP: `DOM.getBoxModel`.
- `screenshot(options)`:
    - element clip geometry + page capture.
    - CDP: `DOM.getBoxModel`/`DOM.getContentQuads` + `Page.captureScreenshot`.
- `waitForElementState(...)`, `waitForSelector(...)`:
    - runtime polling/actionability checks.
    - CDP: `Runtime.callFunctionOn` (+ DOM helpers).

### 11.9 `JSHandle` methods

- `evaluate(...)`, `evaluateHandle(...)`:
    - CDP: `Runtime.callFunctionOn` (and `Runtime.evaluate` in direct expression cases).
- `getProperty(name)`, `getProperties()`:
    - CDP: `Runtime.getProperties`.
- `jsonValue()`:
    - runtime value serialization path (`callFunctionOn`/by-value return).
- `dispose()`:
    - CDP: `Runtime.releaseObject`.

### 11.10 Network object methods (`Request`/`Response`/`Route`/`WebSocket`)

- `Request` getters (`url`, `method`, `headers`, timing, redirects, etc.):
    - populated from CDP events: `Network.requestWillBeSent`, `Network.requestWillBeSentExtraInfo`.
- `Response` metadata getters:
    - populated from `Network.responseReceived`, `Network.responseReceivedExtraInfo`, `Network.loadingFinished/Failed`.
- `Response.body()`:
    - CDP: `Network.getResponseBody`.
    - fallback path used in some cases: `Network.loadNetworkResource` + `IO.read` + `IO.close`.
- `Route.continue()`:
    - CDP: `Fetch.continueRequest`.
- `Route.fulfill()`:
    - CDP: `Fetch.fulfillRequest`.
- `Route.abort()`:
    - CDP: `Fetch.failRequest`.
- `Route.fallback()`:
    - Playwright-side reroute policy; eventual terminal action still ends at one of `Fetch.continueRequest` / `Fetch.fulfillRequest` / `Fetch.failRequest`.
- `WebSocket` events/waits:
    - CDP event sources: `Network.webSocketCreated`, `Network.webSocketWillSendHandshakeRequest`, `Network.webSocketHandshakeResponseReceived`, `Network.webSocketFrameSent`, `Network.webSocketFrameReceived`, `Network.webSocketClosed`, `Network.webSocketFrameError`.

### 11.11 Input helper classes (`Keyboard` / `Mouse` / `Touchscreen`)

- `Keyboard.down`, `Keyboard.up`, `Keyboard.press`, `Keyboard.type`:
    - CDP: `Input.dispatchKeyEvent`.
- `Keyboard.insertText`:
    - CDP: `Input.insertText`.
- `Mouse.move`, `Mouse.down`, `Mouse.up`, `Mouse.click`, `Mouse.dblclick`, `Mouse.wheel`:
    - CDP: `Input.dispatchMouseEvent`.
- `Touchscreen.tap`:
    - CDP: `Input.dispatchTouchEvent` (`touchStart` then `touchEnd`).

### 11.12 `Coverage` methods

- `startJSCoverage()`:
    - CDP: `Profiler.enable`, `Profiler.startPreciseCoverage`, `Debugger.enable`, `Debugger.setSkipAllPauses`.
- `stopJSCoverage()`:
    - CDP: `Profiler.takePreciseCoverage`, `Profiler.stopPreciseCoverage`, `Profiler.disable`, `Debugger.disable`.
    - source text fetch: `Debugger.getScriptSource`.
- `startCSSCoverage()`:
    - CDP: `DOM.enable`, `CSS.enable`, `CSS.startRuleUsageTracking`.
- `stopCSSCoverage()`:
    - CDP: `CSS.stopRuleUsageTracking`, `CSS.getStyleSheetText`, `CSS.disable`, `DOM.disable`.

### 11.13 `Tracing` methods

- browser tracing (`Browser.startTracing` / `Browser.stopTracing` and tracing channel flows):
    - CDP: `Tracing.start`, `Tracing.end`, stream retrieval with `IO.read` + `IO.close`.

### 11.14 Methods that are intentionally not CDP-backed

- `APIRequest` / `APIRequestContext` / `APIResponse`:
    - server-side HTTP client pipeline, not browser CDP.
- Selector composition helpers (`selectors.*`, locator string constructors) and timeout/config setters:
    - local/client logic; CDP only when a concrete frame/page operation executes.

### 11.15 Explicit local-only method families (no direct CDP send)

- `BrowserType`: `executablePath`, `name`, `launchServer`.
- `Browser`: `browserType`, `contexts`, `version`, `isConnected`.
- `BrowserContext`: `browser`, `pages`, `backgroundPages`, `serviceWorkers`, timeout setters.
- `Page`: frame/locator factory accessors (`mainFrame`, `frame`, `frames`, `locator`, `getBy*`, `frameLocator`), plus timeout setters and state getters (`isClosed`, `workers`, `video`, `url`).
- `Frame`: structure/identity helpers (`name`, `url`, `parentFrame`, `childFrames`, `isDetached`, selector factory methods).
- `Locator`/`FrameLocator`: selector composition and metadata methods until an action/assertion/read is invoked.
