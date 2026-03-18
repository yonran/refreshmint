use app_lib::scrape::{self, ScrapeConfig};
use std::error::Error;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

const EXTENSION_NAME: &str = "smoke";
const LOGIN_NAME: &str = "smoke-account";
const DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("integration smoke start");
  const url = await page.url();
  refreshmint.reportValue("url", String(url));
  const evalResult = await page.evaluate("1 + 1");
  refreshmint.reportValue("eval", String(evalResult));
  await refreshmint.saveResource("smoke.bin", [111, 107]);
  refreshmint.log("integration smoke done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("integration smoke error: " + msg);
  throw e;
}
"##;

const POPUP_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("integration popup start");
  await page.goto(__OPENER_URL__);
  const opener = page;
  const openerBefore = await opener.url();
  const popupPromise = opener.waitForEvent("popup", 10000);
  await page.evaluate(`(() => {
    document.getElementById('open').click();
    return "clicked";
  })()`);
  const popup = await popupPromise;
  await popup.waitForLoadState("domcontentloaded", 10000);
  const pages = await browser.pages();
  if (!Array.isArray(pages) || pages.length < 2) {
    throw new Error(`expected at least 2 pages, got ${Array.isArray(pages) ? pages.length : 'non-array'}`);
  }
  const marker = await popup.evaluate("document.getElementById('popup-marker') ? 'yes' : 'no'");
  if (marker !== "yes") {
    throw new Error("popup page did not contain expected marker");
  }
  const openerAfter = await opener.url();
  if (openerAfter !== openerBefore) {
    throw new Error(`opener page changed unexpectedly: ${openerAfter}`);
  }
  await refreshmint.saveResource("popup.bin", [111, 107]);
  refreshmint.log("integration popup done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("integration popup error: " + msg);
  throw e;
}
"##;

const POPUP_CLOSE_WAITER_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("popup close waiter test start");
  await page.goto(__OPENER_URL__);
  const popup = await Promise.all([
    page.waitForEvent("popup", 10000),
    page.evaluate(`document.getElementById('open').click()`),
  ]).then(([popup]) => popup);

  await popup.waitForLoadState("domcontentloaded", 10000);

  const waitForResponse = popup
    .waitForResponse("**/never", { timeout: 10000 })
    .then(() => "resolved-unexpectedly")
    .catch(error => String(error && error.message ? error.message : error));
  await popup.evaluate("window.close()");
  const errorMessage = await waitForResponse;

  if (!errorMessage.includes("TargetClosedError")) {
    throw new Error(`expected TargetClosedError after popup close, got: ${errorMessage}`);
  }

  await refreshmint.saveResource("popup_close_waiter.bin", [111, 107]);
  refreshmint.log("popup close waiter test done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("popup close waiter test error: " + msg);
  throw e;
}
"##;

const OVERLAY_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("integration overlay start");
  const blockedHtml = encodeURIComponent(`
    <style>
      #target {
        position: fixed;
        top: 40px;
        left: 40px;
        z-index: 1;
      }
      #overlay {
        position: fixed;
        inset: 0;
        z-index: 2;
        background: rgba(0, 0, 0, 0.1);
      }
    </style>
    <button id="target">Target</button>
    <div id="overlay"></div>
  `);
  await page.goto(`data:text/html,${blockedHtml}`);
  let sawInterceptError = false;
  try {
    await page.click("#target");
  } catch (e) {
    const msg = String(e && e.message ? e.message : e);
    if (msg.includes("intercepts pointer events")) {
      sawInterceptError = true;
    }
  }
  if (!sawInterceptError) {
    throw new Error("expected click to fail with overlay interception error");
  }
  await refreshmint.saveResource("overlay.bin", [111, 107]);
  refreshmint.log("integration overlay done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("integration overlay error: " + msg);
  throw e;
}
"##;

const FRAME_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("frame test start");
  await page.goto(__FRAME_URL__);
  await page.waitForLoadState("domcontentloaded", undefined);

  // 1. frames() should return the main frame and the iframe.
  const framesJson = await page.frames();
  const frames = JSON.parse(framesJson);
  refreshmint.reportValue("frame_count", String(frames.length));
  if (frames.length < 2) {
    throw new Error("Expected at least 2 frames, got " + frames.length + ": " + framesJson);
  }
  const iframeFrame = frames.find(f => f.name === "logonbox");
  if (!iframeFrame) {
    throw new Error("Could not find logonbox frame. Frames: " + framesJson);
  }
  refreshmint.log("frame test: found iframe frame " + JSON.stringify(iframeFrame));

  // 2. isVisible in main frame should NOT see #user (it lives in the iframe).
  refreshmint.log("frame test: checking main-frame visibility");
  const visibleInMain = await page.isVisible("#user");
  refreshmint.reportValue("visible_in_main", String(visibleInMain));
  if (visibleInMain) {
    throw new Error("isVisible('#user') should be false in main frame");
  }

  // 3. Switch to frame by name and verify element methods see iframe content.
  refreshmint.log("frame test: switching to iframe");
  await page.switchToFrame("logonbox");

  refreshmint.log("frame test: checking iframe visibility");
  const visibleInFrame = await page.isVisible("#user");
  refreshmint.reportValue("visible_in_frame", String(visibleInFrame));
  if (!visibleInFrame) {
    throw new Error("isVisible('#user') should be true in logonbox frame");
  }

  refreshmint.log("frame test: evaluating inside iframe");
  const evalInFrame = await page.evaluate("document.getElementById('user') ? 'found' : 'missing'");
  refreshmint.reportValue("eval_in_frame", evalInFrame);
  if (evalInFrame !== "found") {
    throw new Error("evaluate in frame returned: " + evalInFrame);
  }

  refreshmint.log("frame test: filling inside iframe");
  await page.fill("#user", "testuser");
  refreshmint.log("frame test: reading filled value");
  const filledValue = await page.evaluate("document.getElementById('user').value");
  refreshmint.reportValue("filled_value", filledValue);
  if (filledValue !== "testuser") {
    throw new Error("fill in frame failed: value is " + filledValue);
  }

  // 4. switchToMainFrame restores the main-frame context.
  refreshmint.log("frame test: switching back to main frame");
  await page.switchToMainFrame();

  const visibleAfter = await page.isVisible("#user");
  refreshmint.reportValue("visible_after_switch", String(visibleAfter));
  if (visibleAfter) {
    throw new Error("isVisible('#user') should be false after switchToMainFrame");
  }

  const mainVisible = await page.isVisible("#main");
  refreshmint.reportValue("main_visible", String(mainVisible));
  if (!mainVisible) {
    throw new Error("isVisible('#main') should be true in main frame");
  }

  await refreshmint.saveResource("frame_test.bin", [111, 107]);
  refreshmint.log("frame test done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("frame test error: " + msg);
  throw e;
}
"##;

const GOTO_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("integration goto start");
  const baseUrl = __GOTO_URL__;

  await page.goto(baseUrl);
  await page.goto(baseUrl);
  const afterSame = await page.url();
  if (afterSame !== baseUrl) {
    throw new Error(`same-url goto landed at unexpected URL: ${afterSame}`);
  }

  const hashFoo = `${baseUrl}#foo`;
  const hashBar = `${baseUrl}#bar`;
  await page.goto(hashFoo);
  const afterHashFoo = await page.url();
  if (afterHashFoo !== hashFoo) {
    throw new Error(`hash goto to #foo landed at unexpected URL: ${afterHashFoo}`);
  }

  await page.goto(hashBar);
  const afterHashBar = await page.url();
  if (afterHashBar !== hashBar) {
    throw new Error(`hash goto to #bar landed at unexpected URL: ${afterHashBar}`);
  }

  await refreshmint.saveResource("goto.bin", [111, 107]);
  refreshmint.log("integration goto done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("integration goto error: " + msg);
  throw e;
}
"##;

const SCREENSHOT_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("integration screenshot start");
  const html = encodeURIComponent(`
    <style>
      body { margin: 0; background: white; }
      #target {
        width: 120px;
        height: 80px;
        margin: 24px;
        background: rgb(255, 0, 0);
        color: white;
      }
    </style>
    <div id="target">shot</div>
  `);
  await page.goto(`data:text/html,${html}`);
  await page.waitForLoadState("domcontentloaded", 10000);

  const pageShot = await page.screenshot({ path: "shots/page.png" });
  if (!(pageShot instanceof Uint8Array) || pageShot.length === 0) {
    throw new Error("page.screenshot did not return Uint8Array bytes");
  }

  const locatorShot = await page.locator("#target").screenshot({ path: "shots/locator.png" });
  if (!(locatorShot instanceof Uint8Array) || locatorShot.length === 0) {
    throw new Error("locator.screenshot did not return Uint8Array bytes");
  }

  const handle = await page.evaluate("document.getElementById('target')");
  const handleShot = await handle.screenshot({ type: "jpeg", quality: 80, path: "shots/handle.jpeg" });
  if (!(handleShot instanceof Uint8Array) || handleShot.length === 0) {
    throw new Error("elementHandle.screenshot did not return Uint8Array bytes");
  }

  await refreshmint.saveResource("screenshot-api.bin", [111, 107]);
  refreshmint.log("integration screenshot done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("integration screenshot error: " + msg);
  throw e;
}
"##;

const NETWORK_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("network api test start");
  refreshmint.log("network api: before promise-all");
  const requestPromise = page.waitForRequest("**/api/echo*", { timeout: 10000 });
  const responsePromise = page.waitForResponse("**/api/echo*", { timeout: 10000 });
  await page.evaluate(`new Promise(resolve =>
    requestAnimationFrame(() =>
      requestAnimationFrame(() => resolve("armed"))
    )
  )`);
  const [request, response] = await Promise.all([
    requestPromise,
    responsePromise,
    page.evaluate(`fetch(__FETCH_URL__, {
      method: "POST",
      headers: {
        "content-type": "text/plain"
      },
      body: JSON.stringify({ hello: "world" })
    }).then(r => r.text()).catch(err => {
      refreshmint.log("network api fetch error: " + String(err));
      throw err;
    })`),
  ]);
  refreshmint.log("network api: after promise-all");

  if (request.url() !== __FETCH_URL__) {
    throw new Error(`unexpected request url: ${request.url()}`);
  }
  if (request.method() !== "POST") {
    throw new Error(`unexpected request method: ${request.method()}`);
  }
  if (request.resourceType() !== "fetch") {
    throw new Error(`unexpected request resource type: ${request.resourceType()}`);
  }

  const requestHeaders = request.headers();
  if (requestHeaders["content-type"] !== "text/plain") {
    throw new Error(`unexpected request headers: ${JSON.stringify(requestHeaders)}`);
  }

  const requestAllHeaders = await request.allHeaders();
  if (requestAllHeaders["content-type"] !== "text/plain") {
    throw new Error(`unexpected request allHeaders: ${JSON.stringify(requestAllHeaders)}`);
  }

  const requestHeaderValue = await request.headerValue("content-type");
  if (requestHeaderValue !== "text/plain") {
    throw new Error(`unexpected request headerValue: ${String(requestHeaderValue)}`);
  }

  const requestHeadersArray = await request.headersArray();
  if (!Array.isArray(requestHeadersArray) || !requestHeadersArray.some(header => header.name === "content-type" && header.value === "text/plain")) {
    throw new Error(`unexpected request headersArray: ${JSON.stringify(requestHeadersArray)}`);
  }

  const postData = await request.postData();
  refreshmint.log("network api: after postData");
  if (postData == null || !postData.includes("\"hello\":\"world\"")) {
    throw new Error(`unexpected postData: ${String(postData)}`);
  }

  const postJson = await request.postDataJSON();
  refreshmint.log("network api: after postDataJSON");
  if (postJson == null || postJson.hello !== "world") {
    throw new Error(`unexpected postDataJSON: ${JSON.stringify(postJson)}`);
  }

  const postBytes = await request.postDataBuffer();
  refreshmint.log("network api: after postDataBuffer");
  if (!(postBytes instanceof Uint8Array) || postBytes.length === 0) {
    throw new Error("postDataBuffer did not return Uint8Array bytes");
  }

  const linkedResponse = await request.response();
  refreshmint.log("network api: after request.response");
  if (linkedResponse == null || linkedResponse.status() !== 200) {
    throw new Error("request.response() did not resolve to the response");
  }

  if (response.url() !== __FETCH_URL__) {
    throw new Error(`unexpected response url: ${response.url()}`);
  }
  if (response.status() !== 200 || !response.ok()) {
    throw new Error(`unexpected response status: ${response.status()} ok=${response.ok()}`);
  }
  if (response.statusText() !== "OK") {
    throw new Error(`unexpected response statusText: ${response.statusText()}`);
  }

  const responseHeaders = response.headers();
  if (!String(responseHeaders["content-type"] || "").includes("application/json")) {
    throw new Error(`unexpected response headers: ${JSON.stringify(responseHeaders)}`);
  }

  const responseAllHeaders = await response.allHeaders();
  if (responseAllHeaders["x-test-reply"] !== "response-header") {
    throw new Error(`unexpected response allHeaders: ${JSON.stringify(responseAllHeaders)}`);
  }

  const responseHeaderValue = await response.headerValue("x-test-reply");
  if (responseHeaderValue !== "response-header") {
    throw new Error(`unexpected response headerValue: ${String(responseHeaderValue)}`);
  }

  const responseHeaderValues = await response.headerValues("x-test-reply");
  if (!Array.isArray(responseHeaderValues) || responseHeaderValues.length !== 1 || responseHeaderValues[0] !== "response-header") {
    throw new Error(`unexpected response headerValues: ${JSON.stringify(responseHeaderValues)}`);
  }

  const responseHeadersArray = await response.headersArray();
  if (!Array.isArray(responseHeadersArray) || !responseHeadersArray.some(header => header.name === "x-test-reply" && header.value === "response-header")) {
    throw new Error(`unexpected response headersArray: ${JSON.stringify(responseHeadersArray)}`);
  }

  const linkedRequest = await response.request();
  refreshmint.log("network api: after response.request");
  if (linkedRequest == null || linkedRequest.method() !== "POST") {
    throw new Error("response.request() did not resolve to the request");
  }

  const timing = linkedRequest.timing();
  if (typeof timing.startTime !== "number" || timing.startTime <= 0) {
    throw new Error(`unexpected request timing startTime: ${JSON.stringify(timing)}`);
  }
  if (typeof timing.requestStart !== "number" || timing.requestStart < 0) {
    throw new Error(`unexpected request timing requestStart: ${JSON.stringify(timing)}`);
  }
  if (typeof timing.responseStart !== "number" || timing.responseStart < 0) {
    throw new Error(`unexpected request timing responseStart: ${JSON.stringify(timing)}`);
  }
  if (typeof timing.responseEnd !== "number" || timing.responseEnd < timing.responseStart) {
    throw new Error(`unexpected request timing responseEnd: ${JSON.stringify(timing)}`);
  }

  const text = await response.text();
  refreshmint.log("network api: after response.text");
  if (!text.includes("\"ok\":true")) {
    throw new Error(`unexpected response text: ${text}`);
  }

  const json = await response.json();
  refreshmint.log("network api: after response.json");
  if (json == null || json.ok !== true || json.method !== "POST") {
    throw new Error(`unexpected response json: ${JSON.stringify(json)}`);
  }

  const body = await response.body();
  refreshmint.log("network api: after response.body");
  if (!(body instanceof Uint8Array) || body.length === 0) {
    throw new Error("response.body() did not return Uint8Array bytes");
  }

  const serverAddr = await response.serverAddr();
  refreshmint.log("network api: after response.serverAddr");
  if (serverAddr == null || typeof serverAddr.ipAddress !== "string" || serverAddr.ipAddress.length === 0 || typeof serverAddr.port !== "number" || serverAddr.port <= 0) {
    throw new Error(`unexpected response serverAddr: ${JSON.stringify(serverAddr)}`);
  }

  const securityDetails = await response.securityDetails();
  refreshmint.log("network api: after response.securityDetails");
  if (securityDetails !== null) {
    throw new Error(`expected null securityDetails for local http fixture: ${JSON.stringify(securityDetails)}`);
  }

  const responseFrame = response.frame();
  const requestFrame = request.frame();
  if (responseFrame == null || requestFrame == null) {
    throw new Error("expected request/response frames to be available");
  }

  const pageFromFrame = responseFrame.page();
  refreshmint.log("network api: after frame.page");
  if ((await pageFromFrame.url()) !== "about:blank") {
    throw new Error("frame.page() did not return the current page");
  }

  await refreshmint.saveResource("network.bin", [111, 107]);
  refreshmint.log("network api test done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("network api test error: " + msg);
  throw e;
}
"##;

const NETWORK_MATCHER_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("network matcher test start");
  const requestByStringPromise = page.waitForRequest("**/api/{echo,drop}", { timeout: 10000 });
  const requestByRegexPromise = page.waitForRequest(/\/api\/echo$/, { timeout: 10000 });
  const requestByPredicatePromise = page.waitForRequest(async request => {
    if (request.method() !== "POST") {
      return false;
    }
    const payload = await request.postDataJSON();
    return payload != null && payload.hello === "matchers";
  }, { timeout: 10000 });
  const responseByStringPromise = page.waitForResponse("**/api/{echo,drop}", { timeout: 10000 });
  const responseByRegexPromise = page.waitForResponse(/\/api\/echo$/, { timeout: 10000 });
  const responseByPredicatePromise = page.waitForResponse(async response => {
    if (!response.ok() || response.status() !== 200) {
      return false;
    }
    const body = await response.json();
    return body != null && body.ok === true && body.method === "POST";
  }, { timeout: 10000 });

  await page.evaluate(`new Promise(resolve =>
    requestAnimationFrame(() =>
      requestAnimationFrame(() => resolve("armed"))
    )
  )`);

  const [requestByString, requestByRegex, requestByPredicate, responseByString, responseByRegex, responseByPredicate] =
    await Promise.all([
      requestByStringPromise,
      requestByRegexPromise,
      requestByPredicatePromise,
      responseByStringPromise,
      responseByRegexPromise,
      responseByPredicatePromise,
      page.evaluate(`fetch(__FETCH_URL__, {
        method: "POST",
        headers: {
          "content-type": "text/plain"
        },
        body: JSON.stringify({ hello: "matchers" })
      }).then(r => r.text())`),
    ]);

  if (requestByString.url() !== __FETCH_URL__) {
    throw new Error(`string request matcher saw unexpected url: ${requestByString.url()}`);
  }
  if (requestByRegex.url() !== __FETCH_URL__) {
    throw new Error(`regex request matcher saw unexpected url: ${requestByRegex.url()}`);
  }
  if (requestByPredicate.url() !== __FETCH_URL__) {
    throw new Error(`predicate request matcher saw unexpected url: ${requestByPredicate.url()}`);
  }
  if (responseByString.url() !== __FETCH_URL__) {
    throw new Error(`string response matcher saw unexpected url: ${responseByString.url()}`);
  }
  if (responseByRegex.url() !== __FETCH_URL__) {
    throw new Error(`regex response matcher saw unexpected url: ${responseByRegex.url()}`);
  }
  if (responseByPredicate.url() !== __FETCH_URL__) {
    throw new Error(`predicate response matcher saw unexpected url: ${responseByPredicate.url()}`);
  }

  await refreshmint.saveResource("network_matchers.bin", [111, 107]);
  refreshmint.log("network matcher test done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("network matcher test error: " + msg);
  throw e;
}
"##;

const NETWORK_EVENT_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("network event api test start");
  const requestPromise = page.waitForEvent("request", 10000);
  const responsePromise = page.waitForEvent("response", 10000);
  await page.evaluate(`new Promise(resolve =>
    requestAnimationFrame(() =>
      requestAnimationFrame(() => resolve("armed"))
    )
  )`);

  const [request, response] = await Promise.all([
    requestPromise,
    responsePromise,
    page.evaluate(`fetch(__FETCH_URL__, {
      method: "POST",
      headers: {
        "content-type": "text/plain"
      },
      body: JSON.stringify({ hello: "events" })
    }).then(r => r.text())`),
  ]);

  if (request.url() !== __FETCH_URL__ || request.method() !== "POST") {
    throw new Error(`unexpected request alias result: ${request.method()} ${request.url()}`);
  }
  if (response.url() !== __FETCH_URL__ || response.status() !== 200) {
    throw new Error(`unexpected response alias result: ${response.status()} ${response.url()}`);
  }

  const text = await response.text();
  if (!text.includes("\"ok\":true")) {
    throw new Error(`unexpected response text via waitForEvent: ${text}`);
  }

  await refreshmint.saveResource("network_event.bin", [111, 107]);
  refreshmint.log("network event api test done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("network event api test error: " + msg);
  throw e;
}
"##;

const NETWORK_EVENT_OPTIONS_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("network event options test start");
  const requestPromise = page.waitForEvent("request", {
    timeout: 10000,
    predicate: async request => {
      if (request.method() !== "POST") {
        return false;
      }
      const payload = await request.postDataJSON();
      return payload != null && payload.hello === "event-options";
    }
  });
  const responsePromise = page.waitForEvent("response", async response => {
    if (response.status() !== 200) {
      return false;
    }
    const body = await response.json();
    return body != null && body.ok === true && body.method === "POST";
  });

  await page.evaluate(`new Promise(resolve =>
    requestAnimationFrame(() =>
      requestAnimationFrame(() => resolve("armed"))
    )
  )`);

  const [request, response] = await Promise.all([
    requestPromise,
    responsePromise,
    page.evaluate(`fetch(__FETCH_URL__, {
      method: "POST",
      headers: {
        "content-type": "text/plain"
      },
      body: JSON.stringify({ hello: "event-options" })
    }).then(r => r.text())`),
  ]);

  if (request.url() !== __FETCH_URL__) {
    throw new Error(`event options request saw unexpected url: ${request.url()}`);
  }
  if (response.url() !== __FETCH_URL__) {
    throw new Error(`event options response saw unexpected url: ${response.url()}`);
  }

  await refreshmint.saveResource("network_event_options.bin", [111, 107]);
  refreshmint.log("network event options test done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("network event options test error: " + msg);
  throw e;
}
"##;

const NETWORK_LIFECYCLE_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("network lifecycle test start");

  const finishedRequestPromise = page.waitForEvent("requestfinished", 10000);
  const responsePromise = page.waitForResponse("**/api/echo*", { timeout: 10000 });
  const failedRequestPromise = page.waitForEvent("requestfailed", 10000);

  await page.evaluate(`new Promise(resolve =>
    requestAnimationFrame(() =>
      requestAnimationFrame(() => resolve("armed"))
    )
  )`);

  await page.evaluate(`fetch(__FETCH_URL__, {
    method: "POST",
    headers: {
      "content-type": "text/plain"
    },
    body: "ok"
  }).then(r => r.text())`);

  await page.evaluate(`fetch(__DROP_URL__, {
    method: "POST",
    body: "boom"
  }).catch(() => "failed-as-expected")`);

  const finishedRequest = await finishedRequestPromise;
  const response = await responsePromise;
  const failedRequest = await failedRequestPromise;

  const finishedFailure = await finishedRequest.failure();
  if (finishedFailure !== null) {
    throw new Error(`requestfinished failure should be null: ${JSON.stringify(finishedFailure)}`);
  }

  const failedFailure = await failedRequest.failure();
  if (failedFailure == null || typeof failedFailure.errorText !== "string" || failedFailure.errorText.length === 0) {
    throw new Error(`requestfailed missing error text: ${JSON.stringify(failedFailure)}`);
  }

  const finishedResult = await response.finished();
  if (finishedResult !== null) {
    throw new Error(`response.finished() should resolve null for success: ${JSON.stringify(finishedResult)}`);
  }

  await refreshmint.saveResource("network_lifecycle.bin", [111, 107]);
  refreshmint.log("network lifecycle test done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("network lifecycle test error: " + msg);
  throw e;
}
"##;

const NETWORK_REDIRECT_DRIVER_SOURCE: &str = r##"
try {
  refreshmint.log("network redirect test start");
  const initialRequestPromise = page.waitForRequest("**/api/redirect-start", { timeout: 10000 });
  const finalRequestPromise = page.waitForRequest("**/api/redirect-final", { timeout: 10000 });
  const finalResponsePromise = page.waitForResponse("**/api/redirect-final", { timeout: 10000 });

  await page.evaluate(`new Promise(resolve =>
    requestAnimationFrame(() =>
      requestAnimationFrame(() => resolve("armed"))
    )
  )`);

  await page.evaluate(`fetch(__REDIRECT_URL__).then(r => r.text())`);

  const initialRequest = await initialRequestPromise;
  const finalRequest = await finalRequestPromise;
  const finalResponse = await finalResponsePromise;

  const initialResponse = await initialRequest.response();
  if (initialResponse == null || initialResponse.status() !== 302 || initialResponse.ok()) {
    throw new Error(`unexpected redirect response: ${initialResponse ? initialResponse.status() : "null"}`);
  }

  const redirectedTo = await initialRequest.redirectedTo();
  if (redirectedTo == null || redirectedTo.url() !== __FINAL_URL__) {
    throw new Error(`redirectedTo mismatch: ${redirectedTo ? redirectedTo.url() : "null"}`);
  }

  const redirectedFrom = await finalRequest.redirectedFrom();
  if (redirectedFrom == null || redirectedFrom.url() !== __REDIRECT_URL__) {
    throw new Error(`redirectedFrom mismatch: ${redirectedFrom ? redirectedFrom.url() : "null"}`);
  }

  const finalLinkedResponse = await finalRequest.response();
  if (finalLinkedResponse == null || finalLinkedResponse.status() !== 200 || !finalLinkedResponse.ok()) {
    throw new Error(`unexpected final linked response: ${finalLinkedResponse ? finalLinkedResponse.status() : "null"}`);
  }

  const responseRequest = await finalResponse.request();
  if (responseRequest == null || responseRequest.url() !== __FINAL_URL__) {
    throw new Error(`response.request mismatch: ${responseRequest ? responseRequest.url() : "null"}`);
  }

  const finalText = await finalResponse.text();
  if (!finalText.includes("\"redirect\":true")) {
    throw new Error(`unexpected final redirect text: ${finalText}`);
  }

  await refreshmint.saveResource("network_redirect.bin", [111, 107]);
  refreshmint.log("network redirect test done");
} catch (e) {
  const msg = (e && (e.stack || e.message)) ? (e.stack || e.message) : String(e);
  refreshmint.log("network redirect test error: " + msg);
  throw e;
}
"##;

struct TestSandbox {
    root: PathBuf,
}

impl TestSandbox {
    fn new(prefix: &str) -> Result<Self, Box<dyn Error>> {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!(
            "refreshmint-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

fn write_fixture_file(
    sandbox: &TestSandbox,
    name: &str,
    contents: &str,
) -> Result<String, Box<dyn Error>> {
    let path = sandbox.path().join(name);
    fs::write(&path, contents)?;
    file_url(&path)
}

fn file_url(path: &Path) -> Result<String, Box<dyn Error>> {
    let absolute = path.canonicalize()?;
    #[cfg(windows)]
    {
        let normalized = absolute.to_string_lossy().replace('\\', "/");
        Ok(format!("file:///{normalized}"))
    }
    #[cfg(not(windows))]
    {
        Ok(format!("file://{}", absolute.to_string_lossy()))
    }
}

impl Drop for TestSandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct HttpFixtureServer {
    base_url: String,
    shutdown_tx: Option<mpsc::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl HttpFixtureServer {
    fn start() -> Result<Self, Box<dyn Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let base_url = format!("http://{}", addr);
        let thread_base_url = base_url.clone();
        let thread_localhost_url = thread_base_url.replacen("127.0.0.1", "localhost", 1);
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let thread = thread::spawn(move || loop {
            if shutdown_rx.try_recv().is_ok() {
                break;
            }
            let stream = match listener.accept() {
                Ok((stream, _addr)) => stream,
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(_) => break,
            };
            let mut stream = stream;
            let mut buf = [0_u8; 8192];
            let read = match stream.read(&mut buf) {
                Ok(read) => read,
                Err(_) => continue,
            };
            let request = String::from_utf8_lossy(&buf[..read]);
            let first_line = request.lines().next().unwrap_or_default();
            let path = first_line.split_whitespace().nth(1).unwrap_or("/");
            eprintln!("[network-fixture] {first_line}");
            if path == "/" {
                let body = "<!doctype html><html><body><h1>network</h1></body></html>";
                let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                let _ = stream.write_all(response.as_bytes());
                continue;
            }
            if path == "/frame-main" {
                let body = format!(
                    "<!doctype html><html><body><div id=\"main\">Main</div><iframe name=\"logonbox\" src={}></iframe></body></html>",
                    serde_json::to_string(&format!("{thread_localhost_url}/frame-child"))
                        .unwrap_or_else(|_| "\"\"".to_string()),
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                continue;
            }
            if path == "/frame-child" {
                let body = "<!doctype html><html><body><input id=\"user\"><input id=\"pass\"><button id=\"submit\">OK</button></body></html>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                continue;
            }
            if path == "/api/echo" {
                let body = r#"{"ok":true,"method":"POST"}"#;
                let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nX-Test-Reply: response-header\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                let _ = stream.write_all(response.as_bytes());
                continue;
            }
            if path == "/api/redirect-start" {
                let response = format!(
                    "HTTP/1.1 302 Found\r\nLocation: {}/api/redirect-final\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    thread_base_url
                );
                let _ = stream.write_all(response.as_bytes());
                continue;
            }
            if path == "/api/redirect-final" {
                let body = r#"{"redirect":true,"ok":true}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                continue;
            }
            if path == "/api/drop" {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                continue;
            }

            let body = "not found";
            let response = format!(
                    "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            let _ = stream.write_all(response.as_bytes());
        });
        Ok(Self {
            base_url,
            shutdown_tx: Some(shutdown_tx),
            thread: Some(thread),
        })
    }
}

impl Drop for HttpFixtureServer {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn find_file_named(root: &Path, target_name: &str) -> Result<Option<PathBuf>, Box<dyn Error>> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_named(&path, target_name)? {
                return Ok(Some(found));
            }
        } else if path.file_name().and_then(|name| name.to_str()) == Some(target_name) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_smoke_driver_writes_output() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping scrape smoke test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("scrape")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;
    fs::write(&driver_path, DRIVER_SOURCE)?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    scrape::run_scrape(config)?;

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("smoke.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_popup_wait_for_event_switches_tab() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping popup scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("scrape-popup")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    let popup_url = write_fixture_file(
        &sandbox,
        "popup.html",
        "<!doctype html><html><body><div id=\"popup-marker\">popup</div></body></html>",
    )?;
    let opener_html = format!(
        "<!doctype html><html><body><button id=\"open\" type=\"button\">Open Popup</button><script>document.getElementById('open').addEventListener('click', () => window.open({}, '_blank'));</script></body></html>",
        serde_json::to_string(&popup_url)?,
    );
    let opener_url = write_fixture_file(&sandbox, "popup-opener.html", &opener_html)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;
    let popup_driver =
        POPUP_DRIVER_SOURCE.replace("__OPENER_URL__", &serde_json::to_string(&opener_url)?);
    fs::write(&driver_path, popup_driver)?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    scrape::run_scrape(config)?;

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("popup.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_screenshot_api_returns_bytes_and_writes_paths() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping screenshot scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("scrape-screenshot")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = driver_path.parent().ok_or("driver path has no parent")?;
    fs::create_dir_all(driver_parent)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;
    fs::write(&driver_path, SCREENSHOT_DRIVER_SOURCE)?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir.clone()),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    scrape::run_scrape(config)?;

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("screenshot-api.bin");
    assert_eq!(fs::read(&output_file)?, b"ok");

    let downloads_root = profile_dir.join("downloads");
    let page_png = find_file_named(&downloads_root, "page.png")?
        .ok_or("page screenshot path was not written")?;
    let locator_png = find_file_named(&downloads_root, "locator.png")?
        .ok_or("locator screenshot path was not written")?;
    let handle_jpeg = find_file_named(&downloads_root, "handle.jpeg")?
        .ok_or("elementHandle screenshot path was not written")?;

    assert!(!fs::read(&page_png)?.is_empty());
    assert!(!fs::read(&locator_png)?.is_empty());
    assert!(!fs::read(&handle_jpeg)?.is_empty());

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_popup_waiter_rejects_when_popup_closes() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping popup close scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("scrape-popup-close")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    let popup_url = write_fixture_file(
        &sandbox,
        "popup-close.html",
        "<!doctype html><html><body><div id=\"popup-close-marker\">popup</div></body></html>",
    )?;
    let opener_html = format!(
        "<!doctype html><html><body><button id=\"open\" type=\"button\">Open Popup</button><script>document.getElementById('open').addEventListener('click', () => window.open({}, '_blank'));</script></body></html>",
        serde_json::to_string(&popup_url)?,
    );
    let opener_url = write_fixture_file(&sandbox, "popup-close-opener.html", &opener_html)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;
    let driver_source = POPUP_CLOSE_WAITER_DRIVER_SOURCE
        .replace("__OPENER_URL__", &serde_json::to_string(&opener_url)?);
    fs::write(&driver_path, driver_source)?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    scrape::run_scrape(config)?;

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("popup_close_waiter.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_click_reports_overlay_interception() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping overlay scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("scrape-overlay")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;
    fs::write(&driver_path, OVERLAY_DRIVER_SOURCE)?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    scrape::run_scrape(config)?;

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("overlay.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_goto_handles_same_url_and_hash_navigation() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping goto scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("scrape-goto")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    let goto_url = write_fixture_file(
        &sandbox,
        "goto.html",
        "<!doctype html><title>goto</title><h1>ok</h1>",
    )?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;
    fs::write(
        &driver_path,
        GOTO_DRIVER_SOURCE.replace("__GOTO_URL__", &serde_json::to_string(&goto_url)?),
    )?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    scrape::run_scrape(config)?;

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("goto.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_frame_methods_switch_context() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping frame scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("scrape-frame")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    let frame_child_url = write_fixture_file(
        &sandbox,
        "frame-child.html",
        "<!doctype html><html><body><input id=\"user\"><input id=\"pass\"><button id=\"submit\">OK</button></body></html>",
    )?;
    let frame_html = format!(
        "<!doctype html><html><body><div id=\"main\">Main</div><iframe name=\"logonbox\" src={}></iframe></body></html>",
        serde_json::to_string(&frame_child_url)?,
    );
    let frame_url = write_fixture_file(&sandbox, "frame.html", &frame_html)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;
    fs::write(
        &driver_path,
        FRAME_DRIVER_SOURCE.replace("__FRAME_URL__", &serde_json::to_string(&frame_url)?),
    )?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    scrape::run_scrape(config)?;

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("frame_test.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_frame_methods_switch_context_cross_origin_oopif() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping cross-origin frame scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let sandbox = TestSandbox::new("scrape-frame-oopif")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;

    let server = HttpFixtureServer::start()?;
    let frame_url = format!("{}/frame-main", server.base_url);

    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;
    fs::write(
        &driver_path,
        FRAME_DRIVER_SOURCE.replace("__FRAME_URL__", &serde_json::to_string(&frame_url)?),
    )?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    scrape::run_scrape(config)?;

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("frame_test.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_network_request_response_api_works() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping network api scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let server = HttpFixtureServer::start()?;
    let sandbox = TestSandbox::new("scrape-network")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;

    let fetch_url = format!("{}/api/echo", server.base_url);
    let driver_source =
        NETWORK_DRIVER_SOURCE.replace("__FETCH_URL__", &serde_json::to_string(&fetch_url)?);
    fs::write(&driver_path, driver_source)?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    eprintln!("network scrape sandbox: {}", sandbox.path().display());
    let (result_tx, result_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = scrape::run_scrape(config).map_err(|err| err.to_string());
        let _ = result_tx.send(result);
    });

    match result_rx.recv_timeout(Duration::from_secs(30)) {
        Ok(Ok(())) => {}
        Ok(Err(err)) => return Err(err.into()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            return Err(format!(
                "network scrape timed out after 30s; sandbox: {}",
                sandbox.path().display()
            )
            .into())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err("network scrape worker disconnected".into())
        }
    }

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("network.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_network_matchers_work() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping network matcher scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let server = HttpFixtureServer::start()?;
    let sandbox = TestSandbox::new("scrape-network-matchers")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;

    let fetch_url = format!("{}/api/echo", server.base_url);
    let driver_source =
        NETWORK_MATCHER_DRIVER_SOURCE.replace("__FETCH_URL__", &serde_json::to_string(&fetch_url)?);
    fs::write(&driver_path, driver_source)?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    eprintln!(
        "network matcher scrape sandbox: {}",
        sandbox.path().display()
    );
    let (result_tx, result_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = scrape::run_scrape(config).map_err(|err| err.to_string());
        let _ = result_tx.send(result);
    });

    match result_rx.recv_timeout(Duration::from_secs(30)) {
        Ok(Ok(())) => {}
        Ok(Err(err)) => return Err(err.into()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            return Err(format!(
                "network matcher scrape timed out after 30s; sandbox: {}",
                sandbox.path().display()
            )
            .into())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err("network matcher scrape worker disconnected".into())
        }
    }

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("network_matchers.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_network_wait_for_event_aliases_work() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping network event scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let server = HttpFixtureServer::start()?;
    let sandbox = TestSandbox::new("scrape-network-event")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;

    let fetch_url = format!("{}/api/echo", server.base_url);
    let driver_source =
        NETWORK_EVENT_DRIVER_SOURCE.replace("__FETCH_URL__", &serde_json::to_string(&fetch_url)?);
    fs::write(&driver_path, driver_source)?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    let (result_tx, result_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = scrape::run_scrape(config).map_err(|err| err.to_string());
        let _ = result_tx.send(result);
    });

    match result_rx.recv_timeout(Duration::from_secs(30)) {
        Ok(Ok(())) => {}
        Ok(Err(err)) => return Err(err.into()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            return Err(format!(
                "network event scrape timed out after 30s; sandbox: {}",
                sandbox.path().display()
            )
            .into())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err("network event scrape worker disconnected".into())
        }
    }

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("network_event.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_network_wait_for_event_options_work() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping network event options scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let server = HttpFixtureServer::start()?;
    let sandbox = TestSandbox::new("scrape-network-event-options")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;

    let fetch_url = format!("{}/api/echo", server.base_url);
    let driver_source = NETWORK_EVENT_OPTIONS_DRIVER_SOURCE
        .replace("__FETCH_URL__", &serde_json::to_string(&fetch_url)?);
    fs::write(&driver_path, driver_source)?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    let (result_tx, result_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = scrape::run_scrape(config).map_err(|err| err.to_string());
        let _ = result_tx.send(result);
    });

    match result_rx.recv_timeout(Duration::from_secs(30)) {
        Ok(Ok(())) => {}
        Ok(Err(err)) => return Err(err.into()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            return Err("network event options scrape timed out after 30s".into());
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err("network event options scrape worker disconnected".into());
        }
    }

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("network_event_options.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_network_lifecycle_events_work() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping network lifecycle scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let server = HttpFixtureServer::start()?;
    let sandbox = TestSandbox::new("scrape-network-lifecycle")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;

    let fetch_url = format!("{}/api/echo", server.base_url);
    let drop_url = format!("{}/api/drop", server.base_url);
    let driver_source = NETWORK_LIFECYCLE_DRIVER_SOURCE
        .replace("__FETCH_URL__", &serde_json::to_string(&fetch_url)?)
        .replace("__DROP_URL__", &serde_json::to_string(&drop_url)?);
    fs::write(&driver_path, driver_source)?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    let (result_tx, result_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = scrape::run_scrape(config).map_err(|err| err.to_string());
        let _ = result_tx.send(result);
    });

    match result_rx.recv_timeout(Duration::from_secs(30)) {
        Ok(Ok(())) => {}
        Ok(Err(err)) => return Err(err.into()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            return Err(format!(
                "network lifecycle scrape timed out after 30s; sandbox: {}",
                sandbox.path().display()
            )
            .into())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err("network lifecycle scrape worker disconnected".into())
        }
    }

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("network_lifecycle.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}

#[test]
#[ignore = "requires a local Chrome/Edge install; run periodically with --ignored"]
fn scrape_network_redirects_work() -> Result<(), Box<dyn Error>> {
    if scrape::browser::find_chrome_binary().is_err() {
        eprintln!("skipping network redirect scrape test: Chrome/Edge binary not found");
        return Ok(());
    }

    let server = HttpFixtureServer::start()?;
    let sandbox = TestSandbox::new("scrape-network-redirect")?;
    let ledger_dir = sandbox.path().join("ledger.refreshmint");
    let driver_path = ledger_dir
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("driver.mjs");
    let driver_parent = match driver_path.parent() {
        Some(parent) => parent,
        None => return Err("driver path has no parent".into()),
    };
    fs::create_dir_all(driver_parent)?;
    fs::write(
        driver_parent.join("manifest.json"),
        format!("{{\"name\":\"{EXTENSION_NAME}\"}}"),
    )?;

    let redirect_url = format!("{}/api/redirect-start", server.base_url);
    let final_url = format!("{}/api/redirect-final", server.base_url);
    let driver_source = NETWORK_REDIRECT_DRIVER_SOURCE
        .replace("__REDIRECT_URL__", &serde_json::to_string(&redirect_url)?)
        .replace("__FINAL_URL__", &serde_json::to_string(&final_url)?);
    fs::write(&driver_path, driver_source)?;

    let profile_dir = sandbox.path().join("profile");
    let config = ScrapeConfig {
        login_name: LOGIN_NAME.to_string(),
        extension_name: EXTENSION_NAME.to_string(),
        ledger_dir: ledger_dir.clone(),
        profile_override: Some(profile_dir),
        prompt_overrides: app_lib::scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    let (result_tx, result_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = scrape::run_scrape(config).map_err(|err| err.to_string());
        let _ = result_tx.send(result);
    });

    match result_rx.recv_timeout(Duration::from_secs(30)) {
        Ok(Ok(())) => {}
        Ok(Err(err)) => return Err(err.into()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            return Err("network redirect scrape timed out after 30s".into());
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err("network redirect scrape worker disconnected".into());
        }
    }

    let output_file = ledger_dir
        .join("cache")
        .join("extensions")
        .join(EXTENSION_NAME)
        .join("output")
        .join("network_redirect.bin");
    let bytes = fs::read(&output_file)?;
    assert_eq!(bytes, b"ok");

    Ok(())
}
