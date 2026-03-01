FROM ubuntu:24.04

ARG DEBIAN_FRONTEND=noninteractive
ARG NODE_MAJOR=20
ARG RUST_TOOLCHAIN=stable
ARG TARGETARCH

ENV CARGO_HOME=/opt/cargo
ENV RUSTUP_HOME=/opt/rustup
ENV PATH=/opt/cargo/bin:${PATH}
ENV CI=1
ENV CARGO_TERM_COLOR=always
ENV WORKDIR=/work/refreshmint

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        git \
        gnupg \
        xz-utils \
        unzip \
        build-essential \
        pkg-config \
        libssl-dev \
        python3 \
        libnspr4 \
        libnss3 \
        libasound2t64 \
        fonts-liberation \
        libatk-bridge2.0-0 \
        libatk1.0-0 \
        libcups2 \
        libdbus-1-3 \
        libdrm2 \
        libgbm1 \
        libglib2.0-0 \
        libgtk-3-0 \
        libpango-1.0-0 \
        libx11-6 \
        libx11-xcb1 \
        libxcb1 \
        libxcomposite1 \
        libxdamage1 \
        libxext6 \
        libxfixes3 \
        libxkbcommon0 \
        libxrandr2 \
        libwebkit2gtk-4.1-dev \
        libgtk-3-dev \
        libayatana-appindicator3-dev \
        librsvg2-dev \
        xvfb \
    && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL https://deb.nodesource.com/setup_${NODE_MAJOR}.x | bash - \
    && apt-get update \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal --default-toolchain ${RUST_TOOLCHAIN}

RUN if [ "${TARGETARCH}" = "amd64" ]; then \
        CHROME_URL="$(python3 -c "import json, urllib.request; data=json.load(urllib.request.urlopen('https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json')); print(next(entry['url'] for entry in data['channels']['Stable']['downloads']['chrome'] if entry['platform'] == 'linux64'))")" \
        && curl -fsSL "$CHROME_URL" -o /tmp/chrome.zip \
        && unzip -q /tmp/chrome.zip -d /opt \
        && ln -sf /opt/chrome-linux64/chrome /usr/local/bin/google-chrome \
        && rm -f /tmp/chrome.zip; \
    else \
        npm install -g playwright \
        && PLAYWRIGHT_BROWSERS_PATH=/opt/playwright-browsers playwright install chromium \
        && BROWSER_BIN="$(find /opt/playwright-browsers -type f -path '*/chrome-linux/chrome' | head -n 1)" \
        && test -n "$BROWSER_BIN" \
        && ln -sf "$BROWSER_BIN" /usr/local/bin/google-chrome; \
    fi \
    && google-chrome --version \
    && node --version \
    && npm --version \
    && cargo --version

WORKDIR ${WORKDIR}

COPY . ${WORKDIR}

RUN npm ci

CMD xvfb-run -a cargo test --manifest-path src-tauri/Cargo.toml --test scrape_integration -- --ignored --test-threads=1
