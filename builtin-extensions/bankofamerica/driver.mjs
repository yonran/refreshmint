// Bank of America scraper for Refreshmint
// State machine: inspects current page and dispatches to the appropriate handler.

function isSecureBofaUrl(url) {
    return url.startsWith('https://secure.bankofamerica.com/');
}

function isCredentialResetUrl(url) {
    return url.startsWith(
        'https://secure.bankofamerica.com/auth/forgot/reset-entry/',
    );
}

function isBofaOriginUrl(url) {
    return (
        url.startsWith('https://www.bankofamerica.com/') ||
        url.startsWith('https://bankofamerica.com/') ||
        isSecureBofaUrl(url)
    );
}

/**
 * @param {unknown} x
 * @return {boolean}
 */
function assertBoolean(x) {
    if (typeof x === 'boolean') {
        return x;
    }
    throw new Error('expected boolean; got ' + typeof x);
}

// Pace automation to reduce anti-bot risk and mimic human cadence.
async function waitMs(ms) {
    var safeMs = Math.max(0, Math.floor(ms));
    await page.evaluate(
        '(async function(ms) { await new Promise(function(resolve) { setTimeout(resolve, ms); }); return true; })(' +
            safeMs +
            ')',
    );
}

async function humanPace(minMs, maxMs) {
    var low = Math.max(0, Math.floor(minMs));
    var high = Math.max(low, Math.floor(maxMs));
    var delta = high - low;
    var ms = low + Math.floor(Math.random() * (delta + 1));
    await waitMs(ms);
}

async function hasBofaLoginDom() {
    var present = await page.evaluate(`(function() {
            var user = document.querySelector('#enterID-input');
            var pass = document.querySelector('#tlpvt-passcode-input');
            return !!(user && pass);
        })()`);
    return assertBoolean(present);
}

function errorWithCause(message, cause) {
    var err;
    try {
        err = new Error(message, { cause: cause });
    } catch (_ctorErr) {
        err = new Error(message);
    }
    if (cause !== undefined && err && typeof err === 'object') {
        try {
            if (!('cause' in err) || err.cause !== cause) {
                err.cause = cause;
            }
        } catch (_assignErr) {
            // Ignore cause-assignment failures on non-extensible errors.
        }
    }
    return err;
}

function formatErrorChain(error, maxDepth) {
    var limit = Math.max(1, Math.floor(maxDepth || 6));
    var out = [];
    var seen = [];
    var current = error;
    for (var depth = 0; depth < limit; depth++) {
        var line = '';
        if (current && typeof current === 'object') {
            var stack = '';
            var name = '';
            var message = '';
            try {
                stack = String(current.stack || '').trim();
            } catch (_stackErr) {
                // Ignore stack read errors and fall back to message/name.
            }
            try {
                name = String(current.name || '').trim();
            } catch (_nameErr) {
                // Ignore name read errors and fall back to remaining fields.
            }
            try {
                message = String(current.message || '').trim();
            } catch (_messageErr) {
                // Ignore message read errors and fall back to remaining fields.
            }
            line =
                stack ||
                (name && message
                    ? name + ': ' + message
                    : name || message || String(current));
        } else {
            line = String(current);
        }
        if (!line) {
            line = '(unknown error)';
        }
        out.push((depth === 0 ? '' : 'Caused by: ') + line);

        if (!(current && typeof current === 'object')) {
            break;
        }
        if (seen.indexOf(current) !== -1) {
            out.push('Caused by: ... (cycle)');
            break;
        }
        seen.push(current);

        var nextCause;
        try {
            nextCause = current.cause;
        } catch (_causeReadErr) {
            nextCause = undefined;
        }
        if (nextCause === undefined) {
            break;
        }
        current = nextCause;
    }
    return out.join('\n');
}

async function handleHomepage() {
    refreshmint.log('State: homepage');
    // Expectation: we are on a Bank of America public/home origin page.
    var currentUrl = await page.url();
    if (!isBofaOriginUrl(currentUrl)) {
        throw new Error(
            'Expected Bank of America homepage, got: ' + currentUrl,
        );
    }

    // Dismiss cookie banner if present
    try {
        await page.evaluate(
            '(function() { var b = document.querySelector("button[aria-label=\\"Close\\"]"); if (b) b.click(); })()',
        );
        refreshmint.log('Dismissed cookie banner');
    } catch (_e) {
        refreshmint.log('No cookie banner');
    }

    // Check for modal/interstitial controls first.
    var modalRes = /** @type {string} */ (
        await page.evaluate(
            '(function() {\
        function isVisible(el) {\
            if (!el) return false;\
            var st = window.getComputedStyle(el);\
            var r = el.getBoundingClientRect();\
            return st && st.visibility !== "hidden" && st.display !== "none" && r.width > 0 && r.height > 0;\
        }\
        var direct = document.querySelector("#getTheAppModalContinue");\
        if (isVisible(direct)) { direct.click(); return "clicked:#getTheAppModalContinue"; }\
        var candidates = Array.from(document.querySelectorAll("button,a,[role=\\"button\\"],input[type=\\"button\\"],input[type=\\"submit\\"]"));\
        for (var i = 0; i < candidates.length; i++) {\
            var el = candidates[i];\
            if (!isVisible(el)) continue;\
            var text = (el.textContent || el.value || "").trim().toLowerCase();\
            if (text === "continue to log in" || text === "continue to login") {\
                el.click();\
                return "clicked modal continue: " + text;\
            }\
        }\
        return "no modal continue found";\
    })()',
        )
    );
    refreshmint.log('Modal result: ' + modalRes);

    refreshmint.log('Clicking Log in control');
    var clickRes = /** @type {string} */ (
        await page.evaluate(
            '(function() {\
        function isVisible(el) {\
            if (!el) return false;\
            var st = window.getComputedStyle(el);\
            var r = el.getBoundingClientRect();\
            return st && st.visibility !== "hidden" && st.display !== "none" && r.width > 0 && r.height > 0;\
        }\
        var selectors = [\
            "a[href*=\\"secure.bankofamerica.com/login\\"]",\
            "a[href*=\\"/login/sign-in\\"]",\
            "a[aria-label*=\\"Log in\\"]",\
            "button[aria-label*=\\"Log in\\"]"\
        ];\
        for (var s = 0; s < selectors.length; s++) {\
            var el = null;\
            try { el = document.querySelector(selectors[s]); } catch (e) { continue; }\
            if (isVisible(el)) { el.click(); return "clicked selector: " + selectors[s]; }\
        }\
        var controls = Array.from(document.querySelectorAll("button,a,[role=\\"button\\"],input[type=\\"button\\"],input[type=\\"submit\\"]"));\
        for (var i = 0; i < controls.length; i++) {\
            var c = controls[i];\
            if (!isVisible(c)) continue;\
            var text = (c.textContent || c.value || "").trim().toLowerCase();\
            if (text === "log in") { c.click(); return "clicked text log in"; }\
        }\
        return "log in control not found";\
    })()',
        )
    );
    refreshmint.log('Click result: ' + clickRes);

    await humanPace(500, 1100);
    refreshmint.log('URL before waitForURL: ' + (await page.url()));
    try {
        await page.waitForURL('*secure.bankofamerica.com*', 30000);
    } catch (e) {
        refreshmint.log('waitForURL threw: ' + e);
        refreshmint.log('Popup events: ' + (await page.popupEvents()));
        if (!isSecureBofaUrl(await page.url())) {
            refreshmint.log('Falling back to direct secure login URL');
            await page.goto(
                'https://secure.bankofamerica.com/login/sign-in/signOnV2Screen.go',
            );
            await page.waitForURL('*secure.bankofamerica.com/login/*', 45000);
        }
    }
    refreshmint.log('After homepage, URL: ' + (await page.url()));
    return false;
}

async function handleLogin() {
    refreshmint.log('State: login page');
    var preSubmitUrl = await page.url();
    var loginDomPresent = false;
    try {
        loginDomPresent = await hasBofaLoginDom();
    } catch (e) {
        refreshmint.log('Login DOM detect warning: ' + e);
    }
    // Expectation: secure sign-in page with username/password inputs.
    if (
        preSubmitUrl.indexOf('/login/') === -1 &&
        preSubmitUrl.indexOf('signOnV2Screen.go') === -1 &&
        !loginDomPresent
    ) {
        throw new Error('Expected login page URL, got: ' + preSubmitUrl);
    }
    if (
        preSubmitUrl.indexOf('/login/') === -1 &&
        preSubmitUrl.indexOf('signOnV2Screen.go') === -1 &&
        loginDomPresent
    ) {
        // UNTESTED: in prebuilt debug sessions URL can remain chrome:// while login DOM is loaded.
        refreshmint.log(
            'Login URL fallback: proceeding via login DOM despite URL ' +
                preSubmitUrl,
        );
    }
    if (!isSecureBofaUrl(preSubmitUrl) && loginDomPresent) {
        throw new Error(
            'Login form is visible but current URL is not secure.bankofamerica.com (' +
                preSubmitUrl +
                '). Secret substitution is blocked in this state; complete login manually in the debug browser once, then rerun debug exec.',
        );
    }

    // Fill username with secret substitution and trigger blur/change
    // without mutating the entered value, so password field becomes interactable.
    await page.fill('#enterID-input', 'bofa_username');
    refreshmint.log('Filled username');
    await page.evaluate(
        '(function() {\
        var el = document.querySelector("#enterID-input");\
        if (!el) return;\
        el.value = (el.value || "").trim();\
        el.dispatchEvent(new Event("input", { bubbles: true }));\
        el.dispatchEvent(new Event("change", { bubbles: true }));\
        el.blur();\
    })()',
    );
    refreshmint.log('Triggered username blur/change');
    var currentUser = await page.inputValue('#enterID-input');
    refreshmint.log(
        'Username placeholder present: ' +
            (currentUser === 'bofa_username') +
            ', length=' +
            currentUser.length,
    );

    var pwEnabled = await page.isEnabled('#tlpvt-passcode-input');
    refreshmint.log('Password enabled: ' + pwEnabled);
    if (!pwEnabled) {
        var retryEnabled = await page.evaluate(
            '(function() {\
            var user = document.querySelector("#enterID-input");\
            var pass = document.querySelector("#tlpvt-passcode-input");\
            if (!user || !pass) return false;\
            user.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", code: "Tab", keyCode: 9, which: 9, bubbles: true }));\
            user.dispatchEvent(new KeyboardEvent("keyup", { key: "Tab", code: "Tab", keyCode: 9, which: 9, bubbles: true }));\
            user.blur();\
            return !pass.disabled;\
        })()',
        );
        pwEnabled = assertBoolean(retryEnabled);
        refreshmint.log('Password enabled after tab/blur retry: ' + pwEnabled);
    }
    if (!pwEnabled) {
        throw new Error(
            'Password field remained disabled after entering username',
        );
    }

    try {
        await page.fill('#tlpvt-passcode-input', 'bofa_password');
        refreshmint.log('Filled password');
        var currentPw = await page.inputValue('#tlpvt-passcode-input');
        refreshmint.log(
            'Password placeholder present: ' +
                (currentPw === 'bofa_password') +
                ', length=' +
                currentPw.length,
        );
    } catch (e) {
        refreshmint.log('fill password threw: ' + e);
        throw errorWithCause('Failed to fill password field', e);
    }

    refreshmint.log('Waiting before submit for manual verification');
    await humanPace(1800, 2800);

    // Submit is an <a> link with text "Log In"
    await page.evaluate(
        '(function() { var links = document.querySelectorAll("a"); for (var i = 0; i < links.length; i++) { if (links[i].textContent.trim() === "Log In") { links[i].click(); return; } } })()',
    );
    refreshmint.log('Clicked Log In');
    await humanPace(600, 1300);

    var postLoginPatterns = [
        '*secure.bankofamerica.com/auth/*',
        '*secure.bankofamerica.com/myaccounts*',
        '*secure.bankofamerica.com/customer-uci/*',
        '*secure.bankofamerica.com/mycommunications/statements*',
        '*secure.bankofamerica.com/login/signIn.go*',
    ];
    var matchedPostLoginPattern = '';
    for (var pi = 0; pi < postLoginPatterns.length; pi++) {
        try {
            await page.waitForURL(postLoginPatterns[pi], 6000);
            matchedPostLoginPattern = postLoginPatterns[pi];
            break;
        } catch (e) {
            refreshmint.log(
                'waitForURL post-login miss [' +
                    postLoginPatterns[pi] +
                    ']: ' +
                    e,
            );
        }
    }
    if (matchedPostLoginPattern) {
        refreshmint.log(
            'Login post-submit matched URL pattern: ' + matchedPostLoginPattern,
        );
    } else {
        refreshmint.log(
            'Login post-submit did not match expected post-login URL patterns',
        );
    }
    var newUrl = await page.url();
    if (newUrl === preSubmitUrl) {
        refreshmint.log('Login submit did not navigate; still at: ' + newUrl);
    }
    refreshmint.log('After login, URL: ' + newUrl);

    if (newUrl.includes('InvalidCredentials')) {
        throw new Error('Login failed: invalid credentials');
    }
    if (isCredentialResetUrl(newUrl)) {
        throw new Error(
            'Login failed: Bank of America redirected to the Forgot User ID & Password reset wizard. Username/password are not accepted; complete the reset flow manually and then retry scrape.',
        );
    }
    return false;
}

function normalizeChoice(input, fallbackValue) {
    var raw = (input || '').trim().toLowerCase();
    return raw || fallbackValue;
}

async function availableMfaMethods() {
    var methodsJson = /** @type {string} */ (
        await page.evaluate(
            '(function() {\
        var out = [];\
        if (document.querySelector("#rbText")) out.push("text");\
        if (document.querySelector("#rbVoice")) out.push("voice");\
        if (document.querySelector("#rbEmail")) out.push("email");\
        var labels = Array.from(document.querySelectorAll("label")).map(function(l) { return (l.textContent || "").toLowerCase(); });\
        if (out.indexOf("email") === -1) {\
            for (var i = 0; i < labels.length; i++) { if (labels[i].indexOf("email") !== -1) { out.push("email"); break; } }\
        }\
        return JSON.stringify(out);\
    })()',
        )
    );
    return JSON.parse(methodsJson);
}

async function selectMfaMethod(methodInput) {
    var methods = await availableMfaMethods();
    if (!methods.length) {
        throw new Error('No MFA methods found on page');
    }

    var method = normalizeChoice(
        methodInput,
        methods.indexOf('text') !== -1 ? 'text' : methods[0],
    );
    if (method === 'phone') {
        method = 'voice';
    }
    if (method === 'sms') {
        method = 'text';
    }
    if (methods.indexOf(method) === -1) {
        throw new Error(
            'Requested MFA method "' +
                method +
                '" not available. Available: ' +
                methods.join(', '),
        );
    }

    if (method === 'text') {
        await page.click('#rbText');
        refreshmint.log('Selected MFA method: text');
        return;
    }
    if (method === 'voice') {
        await page.click('#rbVoice');
        refreshmint.log('Selected MFA method: voice');
        return;
    }
    if (method === 'email') {
        var emailSelected = /** @type {boolean} */ (
            await page.evaluate(
                '(function() {\
            var el = document.querySelector("#rbEmail") || document.querySelector("input[id*=email i][type=radio]") || document.querySelector("input[value*=email i][type=radio]");\
            if (el) { el.click(); return true; }\
            var labels = Array.from(document.querySelectorAll("label"));\
            for (var i = 0; i < labels.length; i++) {\
                var text = (labels[i].textContent || "").toLowerCase();\
                if (text.indexOf("email") === -1) continue;\
                var targetId = labels[i].getAttribute("for");\
                if (targetId) {\
                    var input = document.getElementById(targetId);\
                    if (input) { input.click(); return true; }\
                }\
                labels[i].click();\
                return true;\
            }\
            return false;\
        })()',
            )
        );
        if (!emailSelected) {
            throw new Error('Unable to select email MFA option');
        }
        refreshmint.log('Selected MFA method: email');
        return;
    }
}

function pad2(value) {
    return value < 10 ? '0' + value : '' + value;
}

function isoToday() {
    var now = new Date();
    return (
        now.getFullYear() +
        '-' +
        pad2(now.getMonth() + 1) +
        '-' +
        pad2(now.getDate())
    );
}

function parseCoverageDateFromText(input) {
    if (!input) return '';
    var text = String(input);
    var monthMap = {
        january: 1,
        february: 2,
        march: 3,
        april: 4,
        may: 5,
        june: 6,
        july: 7,
        august: 8,
        september: 9,
        october: 10,
        november: 11,
        december: 12,
    };

    var nameMatch = text.match(
        /(January|February|March|April|May|June|July|August|September|October|November|December)\s+(\d{1,2}),\s*(\d{4})/i,
    );
    if (nameMatch) {
        var monthName = nameMatch[1].toLowerCase();
        var month = monthMap[monthName];
        var day = parseInt(nameMatch[2], 10);
        var year = parseInt(nameMatch[3], 10);
        if (month && day >= 1 && day <= 31 && year >= 2000) {
            return year + '-' + pad2(month) + '-' + pad2(day);
        }
    }

    var slashMatch = text.match(/(\d{1,2})\/(\d{1,2})\/(\d{4})/);
    if (slashMatch) {
        var m = parseInt(slashMatch[1], 10);
        var d = parseInt(slashMatch[2], 10);
        var y = parseInt(slashMatch[3], 10);
        if (m >= 1 && m <= 12 && d >= 1 && d <= 31 && y >= 2000) {
            return y + '-' + pad2(m) + '-' + pad2(d);
        }
    }
    return '';
}

function statementCoverageDateFromText(rowText, yearContext) {
    var direct = parseCoverageDateFromText(rowText);
    if (direct) {
        return direct;
    }
    var text = String(rowText || '');
    var lower = text.toLowerCase();

    var year = 0;
    var yearMatch = text.match(/\b(20\d{2})\b/);
    if (yearMatch) {
        year = parseInt(yearMatch[1], 10);
    } else if (yearContext) {
        year = parseInt(yearContext, 10);
    }
    if (!(year >= 2000 && year <= 2100)) {
        return '';
    }

    if (
        lower.indexOf('year-end summary') !== -1 ||
        lower.indexOf('year end summary') !== -1
    ) {
        return year + '-12-31';
    }

    var monthMap = {
        january: 1,
        february: 2,
        march: 3,
        april: 4,
        may: 5,
        june: 6,
        july: 7,
        august: 8,
        september: 9,
        october: 10,
        november: 11,
        december: 12,
    };
    var monthMatch = text.match(
        /\b(January|February|March|April|May|June|July|August|September|October|November|December)\b/i,
    );
    if (!monthMatch) {
        return '';
    }
    var month = monthMap[monthMatch[1].toLowerCase()];
    if (!month) {
        return '';
    }
    var lastDay = new Date(year, month, 0).getDate();
    return year + '-' + pad2(month) + '-' + pad2(lastDay);
}

function filenameCoveragePrefix(filename) {
    if (!filename) return '';
    var m = String(filename).match(/^(\d{4}-\d{2}-\d{2})-/);
    return m ? m[1] : '';
}

function maxIsoDate(a, b) {
    if (!a) return b || '';
    if (!b) return a;
    return a > b ? a : b;
}

function markPdfCoverage(existing, coverage) {
    if (!coverage) return;
    existing.pdfCoverageSet[coverage] = true;
    existing.latestPdfCoverage = maxIsoDate(
        existing.latestPdfCoverage,
        coverage,
    );
}

async function isStatementUnavailableMessageVisible() {
    var unavailable = await page.evaluate(`(function() {
            var text = ((document.body && document.body.innerText) || '')
                .replace(/\\s+/g, ' ')
                .toLowerCase();
            return text.indexOf('the document you are trying to view is not available') !== -1;
        })()`);
    return assertBoolean(unavailable);
}

async function dismissStatementUnavailableMessageIfPossible() {
    var result = /** @type {string} */ (
        await page.evaluate(`(function() {
            function isVisible(el) {
                if (!el) return false;
                var st = window.getComputedStyle(el);
                var r = el.getBoundingClientRect();
                return st && st.visibility !== 'hidden' && st.display !== 'none' && r.width > 0 && r.height > 0;
            }
            var controls = Array.from(document.querySelectorAll('button,a,[role="button"],input[type="button"],input[type="submit"]'));
            for (var i = 0; i < controls.length; i++) {
                var el = controls[i];
                if (!isVisible(el)) continue;
                var text = (el.textContent || el.value || el.getAttribute('aria-label') || '').toLowerCase().trim();
                var id = (el.id || '').toLowerCase();
                if (text === 'ok' || text === 'close' || text === 'cancel' || id.indexOf('close') !== -1) {
                    el.click();
                    return text || id || 'clicked-dismiss-control';
                }
            }
            return '';
        })()`)
    );
    return String(result || '');
}

async function loadExistingDocumentIndex() {
    // UNTESTED: this depends on newly added refreshmint.listAccountDocuments().
    var docsJson = await refreshmint.listAccountDocuments();
    var docs = JSON.parse(docsJson || '[]');
    var latestCsvCoverage = '';
    var latestPdfCoverage = '';
    var csvCoverageSet = {};
    var pdfCoverageSet = {};

    for (var i = 0; i < docs.length; i++) {
        var doc = docs[i] || {};
        var filename = String(doc.filename || '');
        var lower = filename.toLowerCase();
        var coverage =
            String(doc.coverageEndDate || '') ||
            filenameCoveragePrefix(filename);
        if (!coverage) continue;
        if (lower.endsWith('.csv')) {
            csvCoverageSet[coverage] = true;
            latestCsvCoverage = maxIsoDate(latestCsvCoverage, coverage);
        } else if (lower.endsWith('.pdf')) {
            pdfCoverageSet[coverage] = true;
            latestPdfCoverage = maxIsoDate(latestPdfCoverage, coverage);
        }
    }

    return {
        latestCsvCoverage: latestCsvCoverage,
        latestPdfCoverage: latestPdfCoverage,
        csvCoverageSet: csvCoverageSet,
        pdfCoverageSet: pdfCoverageSet,
    };
}

async function handleMfaChoice() {
    refreshmint.log('State: MFA page (signOnSuccessRedirect)');
    // Expectation: security challenge page where user picks delivery method or enters code.
    var url = await page.url();
    if (
        url.indexOf('signOnSuccessRedirect.go') === -1 &&
        url.indexOf('/auth/') === -1
    ) {
        throw new Error('Expected MFA choice page URL, got: ' + url);
    }
    refreshmint.log(await page.snapshot({}));
    var hasMethodSelect = await page.isVisible('#rbText');
    refreshmint.log('Has method selector: ' + hasMethodSelect);

    if (hasMethodSelect) {
        // Step 1: choose a method and request code.
        var methodChoice = await refreshmint.prompt(
            'Choose MFA method (text/voice/email):',
        );
        await selectMfaMethod(methodChoice);
        await page.click('#btnARContinue');
        refreshmint.log('Clicked Continue — code sent');
        return false;
    } else {
        // Step 2: code entry — prompt user
        var code = await refreshmint.prompt('Enter MFA code:');
        await page.fill('#tlpvt-acw-authnum', code);
        refreshmint.log('Filled MFA code');
        // Always remember this device.
        var remember = true;
        try {
            await page.click(remember ? '#yes-recognize' : '#no-recognize');
            refreshmint.log(
                'Selected remember device: ' + (remember ? 'yes' : 'no'),
            );
        } catch (e) {
            refreshmint.log('remember click error: ' + e);
        }
        // Click the continue link to submit MFA code
        await page.click('#continue-auth-number');
        refreshmint.log('Clicked continue-auth-number');
        // Wait for navigation to accounts page
        try {
            await page.waitForURL('*myaccounts*', 30000);
            refreshmint.log('Navigated to accounts!');
            return false;
        } catch (e) {
            var afterSubmitUrl = await page.url();
            if (afterSubmitUrl.includes('/auth/security-center/')) {
                refreshmint.log(
                    'MFA submit landed on security center; returning to top-level dispatcher',
                );
                return false;
            }
            refreshmint.log('waitForURL error (may be wrong code): ' + e);
            refreshmint.log('Current URL: ' + afterSubmitUrl);
            refreshmint.log('Page state after submit:');
            refreshmint.log(await page.snapshot({}));
            return false;
        }
    }
}

async function handleMfa() {
    refreshmint.log('State: MFA verification');
    // Expectation: challenge/verification page with MFA code input.
    var url = await page.url();
    if (
        url.indexOf('verify') === -1 &&
        url.indexOf('challengeQuestion') === -1 &&
        url.indexOf('AuthenticateUser') === -1
    ) {
        throw new Error('Expected MFA verification URL, got: ' + url);
    }
    refreshmint.log('Heading: ' + (await page.innerText('h1, h2')));

    var inputs = /** @type {string} */ (
        await page.evaluate(
            'Array.from(document.querySelectorAll("input")).map(function(i) { return i.id + "[" + i.type + "]"; }).join(", ")',
        )
    );
    refreshmint.log('MFA inputs: ' + inputs);

    var code = await refreshmint.prompt('Enter MFA code:');
    await page.fill('#tlpvt-challenge-answer', code);
    refreshmint.log('Filled MFA code');

    // Always remember this device.
    var remember = true;
    try {
        await page.evaluate(
            '(function(rememberYes) {\
            var selector = rememberYes ? "input[name=\\"rememberComputer\\"][value=\\"yes\\"]" : "input[name=\\"rememberComputer\\"][value=\\"no\\"]";\
            var r = document.querySelector(selector);\
            if (r) r.click();\
        })(' +
                (remember ? 'true' : 'false') +
                ')',
        );
        refreshmint.log(
            'Selected remember device: ' + (remember ? 'yes' : 'no'),
        );
    } catch (_e) {
        refreshmint.log('No remember computer option');
    }

    await page.evaluate(
        '(function() { var a = Array.from(document.querySelectorAll("a")).find(function(a) { return a.textContent.trim() === "SUBMIT"; }); if (a) a.click(); else throw new Error("No SUBMIT link"); })()',
    );
    refreshmint.log('Clicked SUBMIT');

    await page.waitForURL('*myaccounts*');
    refreshmint.log('After MFA, URL: ' + (await page.url()));
    return false;
}

async function handleAccountActivity() {
    refreshmint.log('State: account activity');
    // Expectation: card account details page with period selectors and transaction download actions.
    var url = await page.url();
    if (url.indexOf('account-details.go') === -1) {
        throw new Error('Expected account activity URL, got: ' + url);
    }
    var adxMatch = url.match(/adx=([^&]+)/);
    if (!adxMatch) {
        throw new Error('Could not find adx in URL: ' + url);
    }
    var adx = adxMatch[1];
    refreshmint.log('adx: ' + adx);
    var existing = await loadExistingDocumentIndex();
    refreshmint.log(
        'Existing coverage: csv=' +
            (existing.latestCsvCoverage || 'none') +
            ', pdf=' +
            (existing.latestPdfCoverage || 'none'),
    );

    // Open transaction download panel when required.
    await page.evaluate(
        '(function() {\
        var links = Array.from(document.querySelectorAll("a,button,[role=\\"button\\"]"));\
        for (var i = 0; i < links.length; i++) {\
            var t = (links[i].textContent || "").toLowerCase();\
            if (t.indexOf("download transactions") !== -1) { links[i].click(); return; }\
        }\
    })()',
    );

    // Get all statement periods from top/bottom dropdowns.
    var periodsJson = /** @type {string} */ (
        await page.evaluate(
            'JSON.stringify((function() {\
        var out = [];\
        var seen = {};\
        function addOptions(sel) {\
            var opts = Array.from(document.querySelectorAll(sel + " option"));\
            for (var i = 0; i < opts.length; i++) {\
                var text = (opts[i].textContent || "").trim();\
                var value = opts[i].value || "";\
                var key = text + "::" + value;\
                if (seen[key]) continue;\
                seen[key] = true;\
                out.push({ text: text, value: value });\
            }\
        }\
        addOptions("#goto_select_trans_top");\
        addOptions("#goto_select_trans_bottom");\
        return out;\
    })())',
        )
    );
    var periods = JSON.parse(periodsJson);
    refreshmint.log('Found ' + periods.length + ' periods');

    var csvDownloaded = 0;
    var csvSkipped = 0;

    for (var i = 0; i < periods.length; i++) {
        if (i > 0) {
            await humanPace(1200, 2400);
        }
        var period = periods[i];
        var label = period.text || '';
        var coverageDate = parseCoverageDateFromText(label);
        var stxMatch = period.value.match(/stx=([^&]+)/);
        var stx = stxMatch ? stxMatch[1] : null;

        var shouldDownload = false;
        var reason = '';
        if (!stx) {
            // Always capture latest activity snapshot.
            coverageDate = coverageDate || isoToday();
            shouldDownload = true;
            reason = 'latest-activity';
        } else if (!coverageDate) {
            // UNTESTED: unknown period formats should likely be downloaded.
            shouldDownload = true;
            reason = 'no-coverage-date';
        } else if (
            !existing.latestCsvCoverage ||
            coverageDate > existing.latestCsvCoverage
        ) {
            shouldDownload = true;
            reason = 'since-last-scrape';
        } else if (!existing.csvCoverageSet[coverageDate]) {
            // Same cutoff window but missing locally.
            shouldDownload = true;
            reason = 'missing-local';
        }

        if (!shouldDownload) {
            csvSkipped += 1;
            continue;
        }

        refreshmint.log(
            'Downloading CSV ' +
                (i + 1) +
                '/' +
                periods.length +
                ': ' +
                label +
                ' [' +
                reason +
                ']',
        );
        var downloadUrl;
        var filename;

        if (!stx) {
            // Current transactions (no stx)
            downloadUrl =
                'https://secure.bankofamerica.com/myaccounts/details/card/download-transactions.go?&adx=' +
                adx +
                '&target=downloadCurrentFromDateList&formatType=csv';
            filename = 'current.csv';
        } else {
            downloadUrl =
                'https://secure.bankofamerica.com/myaccounts/details/card/download-transactions.go?&adx=' +
                adx +
                '&stx=' +
                stx +
                '&target=downloadStmtFromDateList&formatType=csv';
            filename = period.text.replace(/[^a-zA-Z0-9]/g, '_') + '.csv';
        }

        // Fetch and encode to bytes in browser context (TextEncoder available there)
        var bytesJson = /** @type {string} */ (
            await page.evaluate(
                '(async function(url) { var r = await fetch(url); var text = await r.text(); return JSON.stringify(Array.from(new TextEncoder().encode(text))); })("' +
                    downloadUrl +
                    '")',
            )
        );
        var bytes = JSON.parse(bytesJson);
        if (!bytes || bytes.length < 10) {
            refreshmint.log('WARNING: Empty response for ' + period.text);
            continue;
        }
        // Decode first 200 chars for logging (ASCII-safe)
        var preview = bytes
            .slice(0, 200)
            .map(function (b) {
                return String.fromCharCode(b);
            })
            .join('');
        var lines = preview.split('\n').length;
        refreshmint.log(
            'Got ' +
                bytes.length +
                ' bytes (~' +
                lines +
                ' lines visible) for ' +
                period.text,
        );

        var csvOptions = {
            coverageEndDate: coverageDate || undefined,
            originalUrl: downloadUrl,
            mimeType: 'text/csv',
        };
        await refreshmint.saveResource(
            'transactions/' + filename,
            bytes,
            csvOptions,
        );
        refreshmint.log('Saved: transactions/' + filename);
        csvDownloaded += 1;
        if (coverageDate) {
            existing.csvCoverageSet[coverageDate] = true;
            existing.latestCsvCoverage = maxIsoDate(
                existing.latestCsvCoverage,
                coverageDate,
            );
        }
    }

    refreshmint.reportValue('csv_downloaded', '' + csvDownloaded);
    refreshmint.reportValue('csv_skipped', '' + csvSkipped);
    await downloadStatementsSinceLastScrape(adx, existing, false);

    refreshmint.reportValue('status', 'ok');
    refreshmint.reportValue('periods_downloaded', '' + periods.length);
    return true;
}

async function openStatementsPage(adx) {
    // UNTESTED: selector-based statements navigation across different BoA layouts.
    var navResult = /** @type {string} */ (
        await page.evaluate(
            '(function() {\
        function isVisible(el) {\
            if (!el) return false;\
            var st = window.getComputedStyle(el);\
            var r = el.getBoundingClientRect();\
            return st && st.visibility !== "hidden" && st.display !== "none" && r.width > 0 && r.height > 0;\
        }\
        var direct = document.querySelector("a[href*=\\"/mycommunications/statements/statement.go\\"]");\
        if (isVisible(direct)) { direct.click(); return "clicked-direct-statements-link"; }\
        var all = Array.from(document.querySelectorAll("a,button,[role=\\"button\\"]"));\
        for (var i = 0; i < all.length; i++) {\
            var t = (all[i].textContent || "").toLowerCase();\
            if (t.indexOf("statements") !== -1 && t.indexOf("documents") !== -1 && isVisible(all[i])) {\
                all[i].click();\
                return "clicked-statements-documents-text";\
            }\
        }\
        return "no-statements-link";\
    })()',
        )
    );
    refreshmint.log('Statements nav result: ' + navResult);
    var redirectUrl =
        'https://secure.bankofamerica.com/myaccounts/brain/redirect.go?target=StatementsCC&adx=' +
        adx +
        '&source=adc';
    var urls = [
        redirectUrl,
        'https://secure.bankofamerica.com/mycommunications/statements/statement.go?&adx=' +
            adx +
            '#!/docs',
        'https://secure.bankofamerica.com/mycommunications/statements/statement.go?&adx=' +
            adx,
        'https://secure.bankofamerica.com/mycommunications/statements/statement.go#!/docs',
        'https://secure.bankofamerica.com/mycommunications/statements/statement.go',
    ];
    if (navResult === 'no-statements-link') {
        refreshmint.log(
            'No in-page statements link found; using direct statements URL',
        );
        await page.goto(redirectUrl);
    }
    try {
        await page.waitForURL('*mycommunications/statements*', 30000);
        return;
    } catch (e) {
        refreshmint.log('Initial waitForURL on statements failed: ' + e);
    }
    // UNTESTED: fallback URL variations for statement app routing differences.
    for (var i = 0; i < urls.length; i++) {
        try {
            await page.goto(urls[i]);
            await humanPace(700, 1500);
            await page.waitForURL('*mycommunications/statements*', 15000);
            refreshmint.log('Statements URL fallback succeeded: ' + urls[i]);
            return;
        } catch (e2) {
            refreshmint.log(
                'Statements URL fallback failed: ' + urls[i] + ' -> ' + e2,
            );
        }
    }
    throw new Error('Unable to open statements page');
}

async function maybeEnterStatementList() {
    // UNTESTED: some BoA views require an extra "Go to My Statements" click.
    try {
        var clicked = /** @type {boolean | string} */ (
            await page.evaluate(`(function() {
                function isVisible(el) {
                    if (!el) return false;
                    var st = window.getComputedStyle(el);
                    var r = el.getBoundingClientRect();
                    return st && st.visibility !== "hidden" && st.display !== "none" && r.width > 0 && r.height > 0;
                }
                var cands = Array.from(document.querySelectorAll('[name="view_your_statements"], a, button, [role="button"]'));
                for (var i = 0; i < cands.length; i++) {
                    var el = cands[i];
                    if (!isVisible(el)) continue;
                    var t = (el.textContent || "").toLowerCase();
                    if (t.indexOf('go to my statement') !== -1 || t.indexOf('view your statement') !== -1) {
                        el.click();
                        return true;
                    }
                }
                return false;
            })()`)
        );
        if (clicked) {
            refreshmint.log('Clicked "Go to My Statements" link');
        }
    } catch (e) {
        refreshmint.log('Statements list-entry click warning: ' + e);
    }
}

async function waitForStatementsContent() {
    // UNTESTED: adaptive wait for SPA-loaded statement controls.
    for (var i = 0; i < 20; i++) {
        var ready = /** @type {boolean} */ (
            await page.evaluate(`(function() {
                function isVisible(el) {
                    if (!el) return false;
                    var st = window.getComputedStyle(el);
                    var r = el.getBoundingClientRect();
                    return st && st.visibility !== "hidden" && st.display !== "none" && r.width > 0 && r.height > 0;
                }
                var hasYear = isVisible(document.querySelector('#yearDropDown'));
                var hasStatementsUrl = window.location.href.indexOf('/mycommunications/statements/statement.go') !== -1;
                var hasPdfDownload = Array.from(document.querySelectorAll('#downloadPDFLink, #downloadPDFAccLink, a, button, [role="button"]')).some(function(el) {
                    if (!isVisible(el)) return false;
                    var text = ((el.textContent || el.value || el.getAttribute('aria-label') || '')).toLowerCase();
                    var id = (el.id || '').toLowerCase();
                    return text.indexOf('download pdf') !== -1 || id.indexOf('downloadpdf') !== -1 || id === 'downloadpdflink' || id === 'downloadpdfacclink';
                });
                return hasStatementsUrl && (hasYear || hasPdfDownload);
            })()`)
        );
        if (ready) {
            return;
        }
        await humanPace(700, 1200);
    }
    refreshmint.log(
        'Statements content wait timed out; continuing with diagnostics',
    );
}

async function listStatementYears() {
    var yearsJson = /** @type {string} */ (
        await page.evaluate(
            `JSON.stringify((function() {
        var out = [];
        var select = document.querySelector('#yearDropDown');
        if (!select) return out;
        var options = Array.from(select.querySelectorAll('option'));
        for (var i = 0; i < options.length; i++) {
            var value = (options[i].value || '').trim();
            var label = (options[i].textContent || '').trim();
            if (!value) continue;
            var year = parseInt(value, 10);
            if (!(year >= 2000 && year <= 2100)) {
                year = parseInt(label, 10);
            }
            if (year >= 2000 && year <= 2100) {
                out.push({ value: value, year: year });
            }
        }
        return out;
    })())`,
        )
    );
    return JSON.parse(yearsJson || '[]');
}

async function selectStatementYear(yearValue) {
    return /** @type {boolean} */ (
        await page.evaluate(`(function(val) {
            var select = document.querySelector('#yearDropDown');
            if (!select) return false;
            select.value = val;
            select.dispatchEvent(new Event('input', { bubbles: true }));
            select.dispatchEvent(new Event('change', { bubbles: true }));
            return true;
        })(${JSON.stringify(yearValue)})`)
    );
}

async function collectStatementEntries() {
    // UNTESTED: statement list scraping depends on dynamic BoA statements DOM.
    var entriesJson = /** @type {string} */ (
        await page.evaluate(
            `JSON.stringify((function() {
        function isVisible(el) {
            if (!el) return false;
            var st = window.getComputedStyle(el);
            var r = el.getBoundingClientRect();
            return st && st.visibility !== "hidden" && st.display !== "none" && r.width > 0 && r.height > 0;
        }
        function cssPath(el) {
            if (!el || el.nodeType !== 1) return "";
            if (el.id) {
                if (document.querySelectorAll("#" + (window.CSS && window.CSS.escape ? window.CSS.escape(el.id) : el.id)).length === 1) {
                    return "#" + (window.CSS && window.CSS.escape ? window.CSS.escape(el.id) : el.id);
                }
            }
            var parts = [];
            var node = el;
            while (node && node.nodeType === 1 && node !== document.body) {
                var name = node.tagName.toLowerCase();
                var idx = 1;
                var sib = node.previousElementSibling;
                while (sib) {
                    if (sib.tagName === node.tagName) idx += 1;
                    sib = sib.previousElementSibling;
                }
                parts.unshift(name + ":nth-of-type(" + idx + ")");
                node = node.parentElement;
            }
            return "body > " + parts.join(" > ");
        }
        function norm(s) {
            return (s || "").replace(/\\s+/g, " ").trim();
        }
        var out = [];
        var seen = {};
        var currentYear = '';
        var yearSelect = document.querySelector('#yearDropDown');
        if (yearSelect) {
            currentYear = yearSelect.value || '';
        }

        var select = null;
        var selects = Array.from(document.querySelectorAll("select"));
        for (var si = 0; si < selects.length; si++) {
            var opts = Array.from(selects[si].querySelectorAll("option")).filter(function(o) {
                var t = (o.textContent || "").toLowerCase();
                return t.indexOf("statement") !== -1 || /\\b\\d{1,2}\\/\\d{1,2}\\/\\d{4}\\b/.test(t) || /\\b\\w+\\s+\\d{1,2},\\s*\\d{4}\\b/.test(t);
            });
            if (opts.length >= 2) {
                select = selects[si];
                break;
            }
        }
        if (select) {
            var options = Array.from(select.querySelectorAll("option"));
            for (var oi = 0; oi < options.length; oi++) {
                var text = norm(options[oi].textContent || "");
                var val = options[oi].value || "";
                if (!val) continue;
                var lower = text.toLowerCase();
                if (lower.indexOf("select") !== -1 || lower.indexOf("choose") !== -1) continue;
                if (lower.indexOf("current") !== -1 || lower.indexOf("today") !== -1) continue;
                out.push({
                    mode: "select",
                    selectSelector: cssPath(select),
                    selectValue: val,
                    rowText: text,
                    clickSelector: null,
                    yearContext: currentYear
                });
            }
        }

        var downloadNodes = Array.from(document.querySelectorAll("a,button,[role=\\"button\\"],input[type=\\"button\\"],input[type=\\"submit\\"]"));
        for (var i = 0; i < downloadNodes.length; i++) {
            var node = downloadNodes[i];
            var text = norm(node.textContent || node.value || node.getAttribute("aria-label") || "");
            var lowerText = text.toLowerCase();
            var id = (node.id || "").toLowerCase();
            var href = (node.getAttribute("href") || "").toLowerCase();
            var cls = (node.className || "").toLowerCase();
            var trigger = (node.getAttribute("data-v-trigger") || "").toLowerCase();
            var explicitPdfAction = trigger.indexOf("downloadpdf") !== -1
                || id === "downloadpdflink"
                || id === "downloadpdfacclink";
            if (!isVisible(node) && !explicitPdfAction) continue;
            var looksDownload = lowerText.indexOf("download") !== -1
                || id.indexOf("download") !== -1
                || href.indexOf("download") !== -1
                || cls.indexOf("download") !== -1
                || trigger.indexOf("downloadpdf") !== -1;
            if (!looksDownload) continue;
            var row = node.closest("tr, li, .row, .statement-row, .statement-item, article, section, div");
            var rowText = norm(row ? row.textContent || "" : text);
            if (rowText.toLowerCase().indexOf('for  for') !== -1) continue;
            if (rowText.length < 12) continue;
            var selector = cssPath(node);
            if (!selector) continue;
            var key = selector + "::" + rowText;
            if (seen[key]) continue;
            seen[key] = true;
            out.push({
                mode: "click",
                selectSelector: null,
                selectValue: null,
                rowText: rowText,
                clickSelector: selector,
                yearContext: currentYear
            });
        }
        return out;
        })())`,
        )
    );
    var raw = JSON.parse(entriesJson || '[]');
    var entries = [];
    for (var i = 0; i < raw.length; i++) {
        var rowText = raw[i].rowText || '';
        entries.push({
            mode: raw[i].mode || 'click',
            selectSelector: raw[i].selectSelector || '',
            selectValue: raw[i].selectValue || '',
            clickSelector: raw[i].clickSelector || '',
            yearContext: raw[i].yearContext || '',
            rowText: rowText,
            coverageDate: statementCoverageDateFromText(
                rowText,
                raw[i].yearContext || '',
            ),
        });
    }
    return entries;
}

function statementFallbackFilename(coverage, index) {
    return 'statement-' + (coverage || index + 1) + '.pdf';
}

function isLikelyPdfUrl(url) {
    var lower = String(url || '').toLowerCase();
    if (!lower) return false;
    if (lower.indexOf('javascript:') === 0) return false;
    return (
        lower.indexOf('.pdf') !== -1 ||
        lower.indexOf('downloadpdf') !== -1 ||
        lower.indexOf('/pdf') !== -1 ||
        lower.indexOf('statement') !== -1
    );
}

async function collectStatementPdfCandidateUrls(
    popupEventsBeforeCount,
    urlBeforeClick,
) {
    var candidates = [];
    var seen = {};
    function add(url) {
        var normalized = String(url || '').trim();
        if (!normalized || seen[normalized]) return;
        seen[normalized] = true;
        candidates.push(normalized);
    }

    try {
        var currentUrl = await page.url();
        if (currentUrl !== urlBeforeClick && isLikelyPdfUrl(currentUrl)) {
            add(currentUrl);
        }
    } catch (_urlErr) {
        // Best-effort URL probe.
    }

    try {
        var popupEvents = JSON.parse((await page.popupEvents()) || '[]');
        for (var i = popupEventsBeforeCount; i < popupEvents.length; i++) {
            var ev = popupEvents[i] || {};
            var evUrl = String(ev.url || '');
            if (isLikelyPdfUrl(evUrl)) {
                add(evUrl);
            }
        }
    } catch (_popupErr) {
        // Best-effort popup event probe.
    }

    try {
        var responses = JSON.parse((await page.networkRequests()) || '[]');
        for (var ri = responses.length - 1; ri >= 0; ri--) {
            var resp = responses[ri] || {};
            var status = Number(resp.status || 0);
            var respUrl = String(resp.url || '');
            if (status >= 200 && status < 400 && isLikelyPdfUrl(respUrl)) {
                add(respUrl);
            }
            if (candidates.length >= 8) break;
        }
    } catch (_respErr) {
        // Best-effort network request probe.
    }

    return candidates;
}

async function trySaveStatementPdfFromUrl(url, filename, coverage) {
    var fetchJson = /** @type {string} */ (
        await page.evaluate(
            '(async function(url) {\
            try {\
                var response = await fetch(url, { credentials: "include" });\
                var contentType = (response.headers.get("content-type") || "").toLowerCase();\
                var buffer = await response.arrayBuffer();\
                var bytes = Array.from(new Uint8Array(buffer));\
                return JSON.stringify({\
                    ok: response.ok,\
                    status: response.status,\
                    contentType: contentType,\
                    bytes: bytes\
                });\
            } catch (e) {\
                return JSON.stringify({ ok: false, error: String(e) });\
            }\
        })(' +
                JSON.stringify(url) +
                ')',
        )
    );
    var fetched = JSON.parse(fetchJson || '{}');
    var bytes = fetched.bytes || [];
    var contentType = String(fetched.contentType || '').toLowerCase();
    var looksPdf =
        contentType.indexOf('application/pdf') !== -1 ||
        (bytes.length >= 4 &&
            bytes[0] === 37 &&
            bytes[1] === 80 &&
            bytes[2] === 68 &&
            bytes[3] === 70);
    if (!fetched.ok || !looksPdf || bytes.length < 128) {
        refreshmint.log(
            'Statement URL fetch was not a PDF [' +
                url +
                ']: ok=' +
                fetched.ok +
                ', status=' +
                fetched.status +
                ', type=' +
                fetched.contentType +
                ', bytes=' +
                bytes.length,
        );
        return false;
    }

    var options = {
        originalUrl: url,
        mimeType: 'application/pdf',
    };
    if (coverage) {
        options.coverageEndDate = coverage;
    }
    await refreshmint.saveResource('statements/' + filename, bytes, options);
    refreshmint.log(
        'Saved statement from URL fallback: statements/' +
            filename +
            (coverage ? ' [' + coverage + ']' : ''),
    );
    return true;
}

async function downloadStatementsSinceLastScrape(
    adx,
    existing,
    alreadyOnStatementsPage,
) {
    var statementsDownloaded = 0;
    var statementsSkipped = 0;
    var statementFailureCount = 0;
    var maxStatementFailureCount = 2;
    var downloadedAnyStatement = false;

    try {
        if (!alreadyOnStatementsPage) {
            await openStatementsPage(adx);
        }
        await maybeEnterStatementList();
        await waitForStatementsContent();
        var statementsUrl = await page.url();
        if (
            adx &&
            statementsUrl.indexOf(
                '/mycommunications/statements/statement.go',
            ) !== -1 &&
            statementsUrl.indexOf('profileEligibilty=') === -1
        ) {
            // UNTESTED: refresh statements context via account redirect to obtain full eligibility context.
            var profileUrl =
                'https://secure.bankofamerica.com/myaccounts/brain/redirect.go?target=StatementsCC&adx=' +
                adx +
                '&source=adc';
            refreshmint.log(
                'Refreshing statements context via redirect URL: ' + profileUrl,
            );
            await page.goto(profileUrl);
            await page.waitForURL('*mycommunications/statements*', 30000);
            await maybeEnterStatementList();
            await waitForStatementsContent();
            statementsUrl = await page.url();
        }
        if (
            statementsUrl.indexOf(
                '/mycommunications/statements/statement.go',
            ) === -1
        ) {
            // UNTESTED: force statements SPA URL if tab click remains on account-details surface.
            var forcedUrl =
                'https://secure.bankofamerica.com/myaccounts/brain/redirect.go?target=StatementsCC&adx=' +
                adx +
                '&source=adc';
            refreshmint.log(
                'Forcing statements URL: ' +
                    forcedUrl +
                    ' from ' +
                    statementsUrl,
            );
            await page.goto(forcedUrl);
            await page.waitForURL('*mycommunications/statements*', 30000);
            await maybeEnterStatementList();
            await waitForStatementsContent();
        }
        refreshmint.log('Statements working URL: ' + (await page.url()));
        // UNTESTED: statement rows often render asynchronously after statements shell URL is loaded.
        await humanPace(2500, 4200);
    } catch (e) {
        refreshmint.log('Statements page open error: ' + e);
        refreshmint.reportValue('statements_downloaded', '0');
        refreshmint.reportValue('statements_skipped', '0');
        return;
    }

    var entries = [];
    var years = await listStatementYears();
    if (years.length) {
        years.sort(function (a, b) {
            return b.year - a.year;
        });
        var latestYear = existing.latestPdfCoverage
            ? parseInt(existing.latestPdfCoverage.slice(0, 4), 10)
            : 0;
        for (var yi = 0; yi < years.length; yi++) {
            var y = years[yi];
            if (latestYear && y.year < latestYear - 1) {
                continue;
            }
            // UNTESTED: year dropdown iteration for pulling older statement PDFs.
            var selected = await selectStatementYear(y.value);
            if (!selected) {
                continue;
            }
            await humanPace(1200, 2200);
            var yearEntries = await collectStatementEntries();
            for (var ye = 0; ye < yearEntries.length; ye++) {
                entries.push(yearEntries[ye]);
            }
        }
    } else {
        entries = await collectStatementEntries();
    }

    var dedup = {};
    var dedupedEntries = [];
    for (var di = 0; di < entries.length; di++) {
        var dk =
            (entries[di].mode || '') +
            '::' +
            (entries[di].selectSelector || '') +
            '::' +
            (entries[di].selectValue || '') +
            '::' +
            (entries[di].clickSelector || '') +
            '::' +
            (entries[di].rowText || '');
        if (dedup[dk]) continue;
        dedup[dk] = true;
        dedupedEntries.push(entries[di]);
    }
    entries = dedupedEntries;

    refreshmint.log('Found statement download entries: ' + entries.length);
    if (!entries.length) {
        // UNTESTED: diagnostics branch for statement-page layouts with unexpected DOM structure.
        try {
            refreshmint.log(
                'Statements snapshot: ' + (await page.snapshot({})),
            );
        } catch (diagErr) {
            refreshmint.log('Statements snapshot error: ' + diagErr);
        }
        try {
            var iframeInfo = /** @type {string} */ (
                await page.evaluate(`(function() {
                    var iframes = Array.from(document.querySelectorAll("iframe"));
                    return JSON.stringify(iframes.map(function(f) {
                        return {
                            id: f.id || "",
                            name: f.name || "",
                            src: f.getAttribute("src") || ""
                        };
                    }));
                })()`)
            );
            refreshmint.log('Statements iframes: ' + iframeInfo);
        } catch (iframeErr) {
            refreshmint.log('Statements iframe inspect error: ' + iframeErr);
        }
    }
    for (var i = 0; i < entries.length; i++) {
        var entry = entries[i];
        var coverage = entry.coverageDate || '';
        var shouldDownload = false;
        if (!coverage) {
            // UNTESTED: no-date statement entries are currently downloaded by default.
            shouldDownload = true;
        } else if (
            !existing.latestPdfCoverage ||
            coverage > existing.latestPdfCoverage
        ) {
            shouldDownload = true;
        } else if (!existing.pdfCoverageSet[coverage]) {
            shouldDownload = true;
        }

        if (!shouldDownload) {
            statementsSkipped += 1;
            continue;
        }
        await humanPace(1800, 3400);

        var download;
        var clickMeta = { ok: false, action: '', href: '', text: '' };
        var urlBeforeStatementClick = await page.url();
        var popupEventsBeforeCount = 0;
        try {
            popupEventsBeforeCount = JSON.parse(
                (await page.popupEvents()) || '[]',
            ).length;
        } catch (_popupEventsReadErr) {
            // Best-effort popup event baseline.
        }
        try {
            await page.clearNetworkRequests();
        } catch (_clearNetworkErr) {
            // Best-effort network reset.
        }

        try {
            if (entry.mode === 'select') {
                // UNTESTED: select-based statement download path for alternate BoA statements UI.
                var clickOk =
                    await page.evaluate(`(function(selectSelector, selectValue) {
                    function isVisible(el) {
                        if (!el) return false;
                        var st = window.getComputedStyle(el);
                        var r = el.getBoundingClientRect();
                        return st && st.visibility !== "hidden" && st.display !== "none" && r.width > 0 && r.height > 0;
                    }
                    var select = document.querySelector(selectSelector);
                    if (!select) return false;
                    select.value = selectValue;
                    select.dispatchEvent(new Event("input", { bubbles: true }));
                    select.dispatchEvent(new Event("change", { bubbles: true }));
                    var controls = Array.from(document.querySelectorAll("a,button,[role=\\"button\\"],input[type=\\"button\\"],input[type=\\"submit\\"]"));
                    for (var i = 0; i < controls.length; i++) {
                        var c = controls[i];
                        if (!isVisible(c)) continue;
                        var text = ((c.textContent || c.value || c.getAttribute("aria-label") || "")).toLowerCase();
                        var href = (c.getAttribute("href") || "").toLowerCase();
                        var id = (c.id || "").toLowerCase();
                        if (text.indexOf("download") !== -1 || text.indexOf("pdf") !== -1
                            || href.indexOf("download") !== -1 || href.indexOf(".pdf") !== -1
                            || id.indexOf("download") !== -1 || id.indexOf("pdf") !== -1) {
                            c.click();
                            return true;
                        }
                    }
                    return false;
                })(${JSON.stringify(entry.selectSelector)}, ${JSON.stringify(entry.selectValue)})`);
                clickMeta.ok = assertBoolean(clickOk);
                clickMeta.action = 'select';
            } else {
                var clickMetaJson = /** @type {string} */ (
                    await page.evaluate(`(function(selector, rowTextHint, ordinal) {
                    function isVisible(el) {
                        if (!el) return false;
                        var st = window.getComputedStyle(el);
                        var r = el.getBoundingClientRect();
                        return st && st.visibility !== "hidden" && st.display !== "none" && r.width > 0 && r.height > 0;
                    }
                    function norm(s) {
                        return (s || "").replace(/\\s+/g, " ").trim();
                    }
                    function pickControl(root, predicate, allowHidden) {
                        if (!root) return null;
                        var controls = Array.from(root.querySelectorAll('a,button,[role="button"],input[type="button"],input[type="submit"]'));
                        for (var i = 0; i < controls.length; i++) {
                            var c = controls[i];
                            if (!allowHidden && !isVisible(c)) continue;
                            if (predicate(c)) return c;
                        }
                        return null;
                    }
                    function controlText(el) {
                        return norm(el && (el.textContent || el.value || el.getAttribute("aria-label") || ""));
                    }
                    function pickByOrdinal(nodes, idx) {
                        if (!nodes || !nodes.length) return null;
                        var n = Number(idx);
                        if (!isFinite(n) || n < 0) n = 0;
                        if (n >= nodes.length) n = nodes.length - 1;
                        return nodes[n];
                    }
                    function forceVisible(el) {
                        var node = el;
                        for (var i = 0; i < 7 && node; i++) {
                            if (node.style) {
                                node.style.display = 'block';
                                node.style.visibility = 'visible';
                                node.style.opacity = '1';
                            }
                            node = node.parentElement;
                        }
                    }
                    var anchor = selector ? document.querySelector(selector) : null;
                    if (!anchor && rowTextHint) {
                        var hint = norm(rowTextHint).toLowerCase();
                        var shortHint = hint.slice(0, 64);
                        var all = Array.from(document.querySelectorAll('a,button,[role="button"],input[type="button"],input[type="submit"]'));
                        for (var ai = 0; ai < all.length; ai++) {
                            var txt = controlText(all[ai]).toLowerCase();
                            if (txt && shortHint && txt.indexOf(shortHint) !== -1) {
                                anchor = all[ai];
                                break;
                            }
                        }
                    }
                    if (!anchor) {
                        var tabs = Array.from(document.querySelectorAll('button,a,[role="button"]'));
                        for (var ti = 0; ti < tabs.length; ti++) {
                            if (!isVisible(tabs[ti])) continue;
                            var tt = controlText(tabs[ti]).toLowerCase();
                            if (tt.indexOf('statements') !== -1) {
                                tabs[ti].click();
                                break;
                            }
                        }
                        var allCtrls = Array.from(document.querySelectorAll('a,button,[role="button"],input[type="button"],input[type="submit"]'));
                        var activateVisible = [];
                        var downloadVisible = [];
                        var downloadAny = [];
                        var viewVisible = [];
                        var viewAny = [];
                        for (var ci = 0; ci < allCtrls.length; ci++) {
                            var c = allCtrls[ci];
                            var t = controlText(c).toLowerCase();
                            var id = (c.id || '').toLowerCase();
                            var trig = (c.getAttribute('data-v-trigger') || '').toLowerCase();
                            var href = (c.getAttribute('href') || '').toLowerCase();
                            var visible = isVisible(c);
                            if (
                                t.indexOf('activate to get view and download links') !== -1 ||
                                (t.indexOf('activate') !== -1 && t.indexOf('view and download') !== -1)
                            ) {
                                if (visible) activateVisible.push(c);
                            }
                            var isDownload =
                                t.indexOf('download pdf') !== -1 ||
                                id.indexOf('downloadpdf') !== -1 ||
                                trig.indexOf('downloadpdf') !== -1 ||
                                href.indexOf('.pdf') !== -1;
                            if (isDownload) {
                                downloadAny.push(c);
                                if (visible) downloadVisible.push(c);
                            }
                            var isView =
                                t.indexOf('view pdf') !== -1 ||
                                id.indexOf('viewpdf') !== -1 ||
                                trig.indexOf('viewpdf') !== -1;
                            if (isView) {
                                viewAny.push(c);
                                if (visible) viewVisible.push(c);
                            }
                        }
                        var act = pickByOrdinal(activateVisible, ordinal);
                        if (act) {
                            act.click();
                        }
                        var genericTarget = pickByOrdinal(downloadVisible, ordinal)
                            || pickByOrdinal(downloadAny, ordinal)
                            || pickByOrdinal(viewVisible, ordinal)
                            || pickByOrdinal(viewAny, ordinal);
                        if (!genericTarget) {
                            return JSON.stringify({ ok: false, action: "not-found", href: "", text: "" });
                        }
                        forceVisible(genericTarget);
                        genericTarget.click();
                        return JSON.stringify({
                            ok: true,
                            action: 'generic-' + (downloadAny.indexOf(genericTarget) !== -1 ? 'download' : 'view'),
                            href: genericTarget.getAttribute('href') || '',
                            text: controlText(genericTarget)
                        });
                    }
                    var row = anchor.closest('tr, li, .statement-item, .document-row, .row, article, section, div');
                    var searchRoot = row || anchor.parentElement || document;
                    var activate = pickControl(searchRoot, function(c) {
                        var t = controlText(c).toLowerCase();
                        return t.indexOf('activate to get view and download links') !== -1
                            || (t.indexOf('activate') !== -1 && t.indexOf('view and download') !== -1);
                    }, false);
                    if (activate) {
                        activate.click();
                    }
                    var downloadControl = pickControl(searchRoot, function(c) {
                        var t = controlText(c).toLowerCase();
                        var id = (c.id || '').toLowerCase();
                        var trig = (c.getAttribute('data-v-trigger') || '').toLowerCase();
                        var href = (c.getAttribute('href') || '').toLowerCase();
                        return t.indexOf('download pdf') !== -1
                            || t.indexOf('download') !== -1
                            || id.indexOf('downloadpdf') !== -1
                            || trig.indexOf('downloadpdf') !== -1
                            || href.indexOf('.pdf') !== -1;
                    }, false) || pickControl(searchRoot, function(c) {
                        var id = (c.id || '').toLowerCase();
                        return id.indexOf('downloadpdf') !== -1;
                    }, true);
                    var clickTarget = downloadControl;
                    var action = 'download';
                    if (!clickTarget) {
                        clickTarget = pickControl(searchRoot, function(c) {
                            var t = controlText(c).toLowerCase();
                            var id = (c.id || '').toLowerCase();
                            var trig = (c.getAttribute('data-v-trigger') || '').toLowerCase();
                            return t.indexOf('view pdf') !== -1
                                || id.indexOf('viewpdf') !== -1
                                || trig.indexOf('viewpdf') !== -1;
                        }, false);
                        action = 'view';
                    }
                    if (!clickTarget) {
                        clickTarget = anchor;
                        action = 'anchor';
                    }
                    var target = row || clickTarget;
                    forceVisible(target);
                    forceVisible(clickTarget);
                    ['mouseover', 'mouseenter', 'mousemove', 'focusin'].forEach(function(evt) {
                        target.dispatchEvent(new MouseEvent(evt, { bubbles: true, cancelable: true, view: window }));
                        clickTarget.dispatchEvent(new MouseEvent(evt, { bubbles: true, cancelable: true, view: window }));
                    });
                    clickTarget.click();
                    return JSON.stringify({
                        ok: true,
                        action: action,
                        href: clickTarget.getAttribute('href') || '',
                        text: controlText(clickTarget)
                    });
                })(${JSON.stringify(entry.clickSelector)}, ${JSON.stringify(entry.rowText || '')}, ${JSON.stringify(i)})`)
                );
                clickMeta = JSON.parse(clickMetaJson || '{}');
            }
            if (!clickMeta.ok) {
                refreshmint.log(
                    'Statement click failed for entry: ' +
                        (entry.rowText || '(no-row-text)'),
                );
                statementFailureCount += 1;
                if (statementFailureCount >= maxStatementFailureCount) {
                    throw new Error(
                        'Statement flow failed repeatedly while clicking download controls',
                    );
                }
                continue;
            }
            refreshmint.log(
                'Statement click action [' +
                    (entry.rowText || '(no-row-text)') +
                    ']: ' +
                    (clickMeta.action || 'unknown') +
                    (clickMeta.href ? ' href=' + clickMeta.href : ''),
            );
            await humanPace(500, 1100);
            if (await isStatementUnavailableMessageVisible()) {
                refreshmint.log(
                    'Statement unavailable from site for entry: ' +
                        (entry.rowText || '(no-row-text)'),
                );
                var dismissResult =
                    await dismissStatementUnavailableMessageIfPossible();
                if (dismissResult) {
                    refreshmint.log(
                        'Dismissed statement unavailable message: ' +
                            dismissResult,
                    );
                }
                throw new Error(
                    'Statement document unavailable from Bank of America for entry: ' +
                        (entry.rowText || '(no-row-text)'),
                );
            }
            try {
                var statementSnapshotDiff = await page.snapshot({
                    incremental: true,
                    track: 'statement-download',
                });
                refreshmint.log(
                    'Statement pre-download snapshot diff [' +
                        (entry.rowText || '(no-row-text)') +
                        ']: ' +
                        statementSnapshotDiff,
                );
            } catch (snapshotDiffErr) {
                refreshmint.log(
                    'Statement pre-download snapshot diff error: ' +
                        snapshotDiffErr,
                );
            }
            download = await page.waitForDownload(5000);
        } catch (e) {
            refreshmint.log('Statement download event failed: ' + e);
        }

        var savedViaUrlFallback = false;
        if (!download) {
            var fallbackSuggested = statementFallbackFilename(coverage, i);
            var candidateUrls = [];
            var candidateSeen = {};
            function addCandidateUrl(url) {
                var normalized = String(url || '').trim();
                if (!normalized || candidateSeen[normalized]) return;
                candidateSeen[normalized] = true;
                candidateUrls.push(normalized);
            }
            if (isLikelyPdfUrl(clickMeta.href)) {
                addCandidateUrl(clickMeta.href);
            }
            var discoveredUrls = await collectStatementPdfCandidateUrls(
                popupEventsBeforeCount,
                urlBeforeStatementClick,
            );
            for (var du = 0; du < discoveredUrls.length; du++) {
                addCandidateUrl(discoveredUrls[du]);
            }
            refreshmint.log(
                'Statement fallback URL candidates [' +
                    (entry.rowText || '(no-row-text)') +
                    ']: ' +
                    candidateUrls.length,
            );
            for (var cu = 0; cu < candidateUrls.length; cu++) {
                var candidate = candidateUrls[cu];
                try {
                    savedViaUrlFallback = await trySaveStatementPdfFromUrl(
                        candidate,
                        fallbackSuggested,
                        coverage,
                    );
                } catch (fallbackErr) {
                    refreshmint.log(
                        'Statement URL fallback error [' +
                            candidate +
                            ']: ' +
                            fallbackErr,
                    );
                }
                if (savedViaUrlFallback) {
                    break;
                }
            }
        }

        if (!download && !savedViaUrlFallback) {
            if (await isStatementUnavailableMessageVisible()) {
                refreshmint.log(
                    'Statement unavailable from site after download timeout for entry: ' +
                        (entry.rowText || '(no-row-text)'),
                );
                var dismissLate =
                    await dismissStatementUnavailableMessageIfPossible();
                if (dismissLate) {
                    refreshmint.log(
                        'Dismissed statement unavailable message: ' +
                            dismissLate,
                    );
                }
                refreshmint.log(
                    'Statement unavailable modal shown by BoA; failing scrape',
                );
                throw new Error(
                    'Statement document unavailable from Bank of America after download timeout for entry: ' +
                        (entry.rowText || '(no-row-text)'),
                );
            }

            statementFailureCount += 1;
            refreshmint.log(
                'No statement download observed for entry: ' +
                    (entry.rowText || '(no-row-text)'),
            );
            if (!downloadedAnyStatement) {
                throw new Error(
                    'Initial statement download smoke test failed: browser did not produce a download',
                );
            }
            if (statementFailureCount >= maxStatementFailureCount) {
                throw new Error(
                    'Statement downloads failed repeatedly: browser did not produce a download',
                );
            }
            continue;
        }

        if (savedViaUrlFallback) {
            downloadedAnyStatement = true;
            statementFailureCount = 0;
            statementsDownloaded += 1;
            markPdfCoverage(existing, coverage);
            continue;
        }

        var suggested =
            download.suggestedFilename ||
            statementFallbackFilename(coverage, i);
        var options = {
            originalUrl: await page.url(),
            mimeType: 'application/pdf',
        };
        if (coverage) {
            options.coverageEndDate = coverage;
        }
        await refreshmint.saveDownloadedResource(
            download.path,
            'statements/' + suggested,
            options,
        );
        refreshmint.log(
            'Saved statement: statements/' +
                suggested +
                (coverage ? ' [' + coverage + ']' : ''),
        );
        downloadedAnyStatement = true;
        statementFailureCount = 0;
        statementsDownloaded += 1;
        markPdfCoverage(existing, coverage);
    }

    refreshmint.reportValue('statements_downloaded', '' + statementsDownloaded);
    refreshmint.reportValue('statements_skipped', '' + statementsSkipped);
}

async function handleStatementsPage() {
    refreshmint.log('State: statements page');
    // Expectation: statements/documents page where we can route to activity or download PDFs.
    var url = await page.url();
    if (url.indexOf('/mycommunications/statements/statement.go') === -1) {
        throw new Error('Expected statements page URL, got: ' + url);
    }
    var adxMatch = url.match(/[?&]adx=([^&]+)/);
    var adx = adxMatch ? adxMatch[1] : '';
    if (!adx) {
        adx = /** @type {string} */ (
            await page.evaluate(`(function() {
            var links = Array.from(document.querySelectorAll('a[href*="adx="]'));
            for (var i = 0; i < links.length; i++) {
                var href = links[i].getAttribute('href') || '';
                var m = href.match(/[?&]adx=([^&]+)/);
                if (m) return m[1];
            }
            return '';
        })()`)
        );
    }
    if (!adx) {
        // UNTESTED: statements-only fallback when adx cannot be recovered.
        var existingNoAdx = await loadExistingDocumentIndex();
        await downloadStatementsSinceLastScrape('', existingNoAdx, true);
        refreshmint.reportValue('status', 'ok');
        return true;
    }
    // UNTESTED: route back to account activity using in-page Activity tab first.
    try {
        var activityClicked = /** @type {boolean | string} */ (
            await page.evaluate(`(function() {
            function isVisible(el) {
                if (!el) return false;
                var st = window.getComputedStyle(el);
                var r = el.getBoundingClientRect();
                return st && st.visibility !== "hidden" && st.display !== "none" && r.width > 0 && r.height > 0;
            }
            var activity = document.querySelector('[name="Activity"]');
            if (isVisible(activity)) {
                activity.click();
                return true;
            }
            var links = Array.from(document.querySelectorAll('a,button,[role="button"]'));
            for (var i = 0; i < links.length; i++) {
                var t = (links[i].textContent || '').toLowerCase().trim();
                if (t === 'activity' && isVisible(links[i])) {
                    links[i].click();
                    return true;
                }
            }
            return false;
        })()`)
        );
        if (activityClicked) {
            refreshmint.log('Clicked Activity tab from statements page');
            await page.waitForURL('*account-details*', 30000);
            return false;
        }
    } catch (activityErr) {
        refreshmint.log(
            'Statements Activity-tab routing warning: ' + activityErr,
        );
    }

    // UNTESTED: direct routing fallback from statements SPA back to account activity.
    var activityUrl =
        'https://secure.bankofamerica.com/myaccounts/details/card/account-details.go?&adx=' +
        adx;
    refreshmint.log(
        'Routing from statements to account activity (URL fallback): ' +
            activityUrl,
    );
    try {
        await page.goto(activityUrl);
        await page.waitForURL('*account-details*', 30000);
        return false;
    } catch (gotoErr) {
        refreshmint.log('Statements->activity URL fallback failed: ' + gotoErr);
    }

    // UNTESTED: if activity routing fails, scrape statements in-place and finish.
    var existing = await loadExistingDocumentIndex();
    await downloadStatementsSinceLastScrape(adx, existing, true);
    refreshmint.reportValue('status', 'ok');
    return true;
}

async function handleAccountsOverview() {
    refreshmint.log('State: accounts overview');
    // Expectation: myaccounts overview with a link to account activity.
    var url = await page.url();
    if (url.indexOf('myaccounts') === -1 && url.indexOf('signIn.go') === -1) {
        throw new Error('Expected accounts overview URL, got: ' + url);
    }
    refreshmint.log(
        'Title: ' +
            /** @type {string} */ (await page.evaluate('document.title')),
    );

    var hasAccount = await page.isVisible('[name="CCA_seeAllTransactions"]');
    refreshmint.log('Has CCA_seeAllTransactions: ' + hasAccount);

    if (hasAccount) {
        await page.click('[name="CCA_seeAllTransactions"]');
        refreshmint.log('Navigating to account activity...');
        try {
            await page.waitForURL('*account-details*', 15000);
            refreshmint.log('At: ' + (await page.url()));
        } catch (e) {
            refreshmint.log(
                'waitForURL error: ' + e + ', URL: ' + (await page.url()),
            );
        }
        return false;
    } else {
        refreshmint.log('No account link found — page snapshot:');
        refreshmint.log(await page.snapshot({}));
        refreshmint.reportValue('status', 'logged_in');
        return true;
    }
}

async function handleSecurityCenter() {
    refreshmint.log('State: security center');
    var url = await page.url();
    // Expectation: post-login security/profile page that should route back to account surfaces.
    if (
        url.indexOf('/auth/security-center/') === -1 &&
        url.indexOf('/customer-uci/') === -1
    ) {
        throw new Error('Expected security-center/profile URL, got: ' + url);
    }
    refreshmint.log('URL: ' + url);
    try {
        // UNTESTED: prefer in-session top-nav accounts routing when available.
        var hasAccountsNav = false;
        try {
            hasAccountsNav = await page.isVisible('[name="onh_accounts"]');
        } catch (e) {
            refreshmint.log('Security-center accounts-nav detect error: ' + e);
        }
        if (hasAccountsNav) {
            try {
                await page.click('[name="onh_accounts"]');
                refreshmint.log('Clicked onh_accounts');
                try {
                    await page.waitForSelector(
                        '[name="onh_accounts_overview"]',
                        5000,
                    );
                    await page.click('[name="onh_accounts_overview"]');
                    refreshmint.log('Clicked onh_accounts_overview');
                } catch (e) {
                    refreshmint.log('accounts_overview click skipped: ' + e);
                }
                await page.waitForURL('*myaccounts*', 30000);
                refreshmint.log(
                    'Navigated from security center to myaccounts via top nav',
                );
                return false;
            } catch (e) {
                refreshmint.log('Top-nav accounts routing error: ' + e);
            }
        }

        // UNTESTED: this branch is tuned from observed security-center redirects
        // and needs verification across multiple post-login variants.
        var targetHref = /** @type {string} */ (
            await page.evaluate(
                '(function() {\
            function isVisible(el) {\
                if (!el) return false;\
                var st = window.getComputedStyle(el);\
                var r = el.getBoundingClientRect();\
                return st && st.visibility !== "hidden" && st.display !== "none" && r.width > 0 && r.height > 0;\
            }\
            var links = Array.from(document.querySelectorAll("a[href*=\\"/myaccounts/\\"]"));\
            var candidates = [];\
            for (var i = 0; i < links.length; i++) {\
                var l = links[i];\
                if (!isVisible(l)) continue;\
                var href = (l.getAttribute("href") || "").toLowerCase();\
                var text = (l.textContent || "").trim().toLowerCase();\
                if (!href) continue;\
                if (href.indexOf("view_contact_details") !== -1) continue;\
                if (href.indexOf("manage-contacts") !== -1) continue;\
                if (href.indexOf("customer-uci") !== -1) continue;\
                if (text.indexOf("contact") !== -1) continue;\
                var score = 0;\
                if (href.indexOf("view_accounts") !== -1) score += 50;\
                if (href.indexOf("overview") !== -1) score += 40;\
                if (href.indexOf("summary") !== -1) score += 30;\
                if (href.indexOf("signin/signin.go") !== -1) score += 20;\
                if (text.indexOf("account") !== -1) score += 10;\
                candidates.push({ href: href, score: score, el: l });\
            }\
            candidates.sort(function(a, b) { return b.score - a.score; });\
            if (candidates.length > 0) { return candidates[0].href; }\
            return "";\
        })()',
            )
        );
        var targetUrl = '';
        if (targetHref) {
            if (
                targetHref.startsWith('http://') ||
                targetHref.startsWith('https://')
            ) {
                targetUrl = targetHref;
            } else if (targetHref.startsWith('/')) {
                targetUrl = 'https://secure.bankofamerica.com' + targetHref;
            }
        }
        if (!targetUrl) {
            targetUrl =
                'https://secure.bankofamerica.com/myaccounts/brain/redirect.go?target=accountsoverview&request_locale=en-us';
        }
        refreshmint.log('Security-center nav target: ' + targetUrl);
        await page.goto(targetUrl);
        await page.waitForURL('*myaccounts*', 30000);
        refreshmint.log('Navigated from security center to myaccounts');
        return false;
    } catch (e) {
        refreshmint.log('Security-center navigation error: ' + e);
    }
    refreshmint.log('Security-center fallback snapshot:');
    refreshmint.log(await page.snapshot({}));
    return false;
}

function urlPathWithHash(url) {
    var text = String(url || '');
    var pathMatch = text.match(/^https?:\/\/[^/]+([^?#]*)/i);
    var path = pathMatch ? pathMatch[1] || '/' : text;
    var hashIndex = text.indexOf('#');
    var hash = hashIndex === -1 ? '' : text.slice(hashIndex);
    return path + hash;
}

function stateTagFromUrl(url) {
    if (!isBofaOriginUrl(url)) {
        return 'offsite';
    }
    if (!isSecureBofaUrl(url)) {
        return 'public-home';
    }
    if (url.includes('signOnSuccessRedirect.go')) {
        return 'mfa-choice';
    }
    if (url.includes('signOnV2Screen.go') || url.includes('/login/')) {
        return 'login';
    }
    if (isCredentialResetUrl(url)) {
        return 'credential-reset';
    }
    if (
        url.includes('verify') ||
        url.includes('challengeQuestion') ||
        url.includes('AuthenticateUser')
    ) {
        return 'mfa-verify';
    }
    if (url.includes('/auth/security-center/')) {
        return 'security-center';
    }
    if (url.includes('/customer-uci/')) {
        return 'profile-center';
    }
    if (url.includes('/mycommunications/statements/statement.go')) {
        return 'statements';
    }
    if (url.includes('account-details.go')) {
        return 'account-activity';
    }
    if (url.includes('myaccounts') || url.includes('signIn.go')) {
        return 'accounts-overview';
    }
    return 'unknown-secure';
}

function stateSignatureFromUrl(url) {
    return stateTagFromUrl(url) + '|' + urlPathWithHash(url);
}

async function dispatchCurrentState() {
    var url = await page.url();
    refreshmint.log('URL: ' + url);

    if (!isBofaOriginUrl(url)) {
        try {
            var loginDomPresent = await hasBofaLoginDom();
            if (loginDomPresent) {
                // UNTESTED: tolerate stale chrome:// URL when secure login DOM is already rendered.
                refreshmint.log(
                    'Offsite URL fallback: login DOM detected; routing as secure login',
                );
                url =
                    'https://secure.bankofamerica.com/login/sign-in/signOnV2Screen.go';
            }
        } catch (domErr) {
            refreshmint.log('Offsite login DOM detect warning: ' + domErr);
        }
    }

    if (!isBofaOriginUrl(url)) {
        try {
            await page.goto('https://www.bankofamerica.com');
        } catch (e) {
            refreshmint.log('goto warning: ' + e);
        }
        try {
            await page.waitForURL('*bankofamerica.com*', 45000);
        } catch (e) {
            refreshmint.log('waitForURL bankofamerica warning: ' + e);
        }
        url = await page.url();
        refreshmint.log('Navigated to: ' + url);

        if (!isBofaOriginUrl(url)) {
            refreshmint.log(
                'Public homepage navigation did not land on BoA origin, trying direct secure login URL',
            );
            try {
                await page.goto(
                    'https://secure.bankofamerica.com/login/sign-in/signOnV2Screen.go',
                );
            } catch (e) {
                refreshmint.log('secure goto warning: ' + e);
            }
            try {
                await page.waitForURL(
                    '*secure.bankofamerica.com/login/*',
                    45000,
                );
            } catch (e) {
                refreshmint.log('waitForURL secure warning: ' + e);
            }
            url = await page.url();
            refreshmint.log('After secure fallback, URL: ' + url);

            if (!isBofaOriginUrl(url)) {
                try {
                    var loginDomPresentLate = await hasBofaLoginDom();
                    if (loginDomPresentLate) {
                        refreshmint.log(
                            'Post-goto offsite URL fallback: login DOM detected after navigation attempts',
                        );
                        url =
                            'https://secure.bankofamerica.com/login/sign-in/signOnV2Screen.go';
                    }
                } catch (domErr2) {
                    refreshmint.log(
                        'Post-goto login DOM detect warning: ' + domErr2,
                    );
                }
            }
        }
    }

    if (!isBofaOriginUrl(url)) {
        throw new Error(
            'Unable to reach bankofamerica.com, current URL: ' + url,
        );
    }

    if (!isSecureBofaUrl(url)) {
        refreshmint.log('-> Dispatching: handleHomepage');
        return await handleHomepage();
    }

    if (url.includes('signOnSuccessRedirect.go')) {
        refreshmint.log('-> Dispatching: handleMfaChoice');
        return await handleMfaChoice();
    }
    if (url.includes('signOnV2Screen.go') || url.includes('/login/')) {
        refreshmint.log('-> Dispatching: handleLogin');
        return await handleLogin();
    }
    if (isCredentialResetUrl(url)) {
        throw new Error(
            'Login failed: Bank of America is showing the Forgot User ID & Password reset wizard. Username/password are not accepted; complete the reset flow manually and then retry scrape.',
        );
    }
    if (
        url.includes('verify') ||
        url.includes('challengeQuestion') ||
        url.includes('AuthenticateUser')
    ) {
        refreshmint.log('-> Dispatching: handleMfa');
        return await handleMfa();
    }
    if (url.includes('/auth/security-center/')) {
        refreshmint.log('-> Dispatching: handleSecurityCenter');
        return await handleSecurityCenter();
    }
    if (url.includes('/customer-uci/')) {
        // UNTESTED: post-login profile/contact paths should route back to account surfaces.
        refreshmint.log('-> Dispatching: handleSecurityCenter (customer-uci)');
        return await handleSecurityCenter();
    }
    if (url.includes('/mycommunications/statements/statement.go')) {
        refreshmint.log('-> Dispatching: handleStatementsPage');
        return await handleStatementsPage();
    }
    if (url.includes('account-details.go')) {
        refreshmint.log('-> Dispatching: handleAccountActivity');
        return await handleAccountActivity();
    }
    if (url.includes('myaccounts') || url.includes('signIn.go')) {
        refreshmint.log('-> Dispatching: handleAccountsOverview');
        return await handleAccountsOverview();
    }

    refreshmint.log('Unknown state — page snapshot:');
    refreshmint.log(await page.snapshot({}));
    return false;
}

// Main: iterate state machine until complete or error.
try {
    refreshmint.log('BOOT: start');
    refreshmint.log('Popup handler: skipped');

    var completed = false;
    var maxIterations = 12;
    var maxIterationsWithoutProgress = 4;
    var iterationsWithoutProgress = 0;
    var lastProgressStep = 0;
    var seenStateSignatures = {};
    for (var step = 1; step <= maxIterations; step++) {
        refreshmint.log(
            '=== State iteration ' + step + '/' + maxIterations + ' ===',
        );
        var beforeUrl = await page.url();
        var beforeSignature = stateSignatureFromUrl(beforeUrl);
        var progressReasons = [];
        if (!seenStateSignatures[beforeSignature]) {
            seenStateSignatures[beforeSignature] = true;
            progressReasons.push('new-state-before');
        }
        refreshmint.log('State signature (before): ' + beforeSignature);

        try {
            completed = await dispatchCurrentState();
        } catch (e) {
            refreshmint.log(
                'State iteration failed at signature: ' + beforeSignature,
            );
            refreshmint.log('State iteration error: ' + e);
            try {
                refreshmint.log(
                    'Failure snapshot: ' + (await page.snapshot({})),
                );
            } catch (snapshotError) {
                refreshmint.log('Failure snapshot error: ' + snapshotError);
            }
            throw errorWithCause(
                'State iteration failed at signature: ' + beforeSignature,
                e,
            );
        }

        var afterUrl = await page.url();
        var afterSignature = stateSignatureFromUrl(afterUrl);
        if (!seenStateSignatures[afterSignature]) {
            seenStateSignatures[afterSignature] = true;
            progressReasons.push('new-state-after');
        }
        if (afterSignature !== beforeSignature) {
            progressReasons.push('state-signature-changed');
        }
        if (afterUrl !== beforeUrl) {
            progressReasons.push('url-changed');
        }
        if (completed) {
            progressReasons.push('completed');
        }
        refreshmint.log('State signature (after): ' + afterSignature);

        if (progressReasons.length) {
            iterationsWithoutProgress = 0;
            lastProgressStep = step;
            refreshmint.log(
                'Progress detected [' +
                    progressReasons.join(', ') +
                    '] at iteration ' +
                    step,
            );
        } else {
            iterationsWithoutProgress += 1;
            refreshmint.log(
                'No progress detected at iteration ' +
                    step +
                    ' (count=' +
                    iterationsWithoutProgress +
                    ')',
            );
            if (iterationsWithoutProgress >= maxIterationsWithoutProgress) {
                // UNTESTED: stall guard for repeated no-progress loops in a single visible state.
                try {
                    refreshmint.log(
                        'Stall snapshot: ' + (await page.snapshot({})),
                    );
                } catch (stallSnapshotError) {
                    refreshmint.log(
                        'Stall snapshot error: ' + stallSnapshotError,
                    );
                }
                throw new Error(
                    'State machine stalled: no progress for ' +
                        iterationsWithoutProgress +
                        ' iterations (last progress at iteration ' +
                        lastProgressStep +
                        ')',
                );
            }
        }

        refreshmint.reportValue('state', afterSignature);
        refreshmint.reportValue(
            'iterations_without_progress',
            '' + iterationsWithoutProgress,
        );

        if (completed) {
            refreshmint.log('State machine completed');
            break;
        }
        await humanPace(900, 1600);
    }

    if (!completed) {
        // UNTESTED: max-iteration guard path should only trigger when flow stalls.
        throw new Error(
            'State machine did not complete within ' +
                maxIterations +
                ' iterations',
        );
    }
} catch (fatalErr) {
    var fatalDetail = '';
    try {
        fatalDetail = formatErrorChain(fatalErr, 10);
    } catch (_stackReadErr) {
        fatalDetail = String(fatalErr);
    }
    throw errorWithCause(
        'Bank of America driver fatal:\n' + fatalDetail,
        fatalErr,
    );
}
