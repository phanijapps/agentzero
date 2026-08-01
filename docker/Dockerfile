# ===========================================================================
# z-Bot Docker Build — Multi-Stage
# Stage 1: Build frontend (Node)
# Stage 2: Build daemon binary (Rust)
# Stage 3: Runtime with tools (Debian)
# ===========================================================================

# ---------------------------------------------------------------------------
# Stage 1: Frontend Build
# ---------------------------------------------------------------------------
FROM node:20-slim AS frontend

WORKDIR /build

# Copy package files first for layer caching
COPY apps/ui/package.json apps/ui/package-lock.json apps/ui/
COPY package.json ./

# Install dependencies
RUN cd apps/ui && npm ci --ignore-scripts

# Copy UI source and build
COPY apps/ui/ apps/ui/
RUN cd apps/ui && npm run build

# ---------------------------------------------------------------------------
# Stage 2: Rust Build
# ---------------------------------------------------------------------------
FROM rust:1.93-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    make \
    gcc \
    g++ \
    libc6-dev \
    libstdc++-12-dev \
    libonig-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy entire workspace and build
# Docker layer caching handles rebuild detection via .dockerignore
COPY . .

RUN cargo build -p daemon --release

# ---------------------------------------------------------------------------
# Stage 3: Runtime
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    # Core tools
    bash \
    curl \
    wget \
    ca-certificates \
    git \
    # Python
    python3 \
    python3-pip \
    python3-venv \
    # LibreOffice (document conversion)
    libreoffice-core \
    libreoffice-writer \
    # SSL and C++ runtime for ONNX
    libssl3 \
    libstdc++6 \
    libonig5 \
    # Cleanup
    && rm -rf /var/lib/apt/lists/* \
    && ln -sf /usr/bin/python3 /usr/bin/python

# Install Node.js 20
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install pnpm via corepack
RUN corepack enable && corepack prepare pnpm@latest --activate

# Install uv (fast Python package manager)
RUN pip3 install --break-system-packages uv

# Copy daemon binary
COPY --from=builder /build/target/release/zbotd /usr/local/bin/zbotd

# Copy built frontend
COPY --from=frontend /build/dist /opt/zbot/ui

# Create non-root user (UID 1000 matches typical host user)
# Running as root causes LLMs to refuse shell commands
RUN useradd -m -u 1000 -s /bin/bash zbot \
    && mkdir -p /data/zbot /home/zbot/.cache/fastembed \
    && chown -R zbot:zbot /data/zbot /opt/zbot /home/zbot

USER zbot
WORKDIR /home/zbot

# Healthcheck
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s \
    CMD curl -f http://localhost:18791/api/health || exit 1

# Client events (`/ws`) and bridge workers (`/bridge/ws`) share the HTTP port.
EXPOSE 18791

ENTRYPOINT ["/usr/local/bin/zbotd"]
CMD ["--host", "0.0.0.0", "--data-dir", "/data/zbot", "--static-dir", "/opt/zbot/ui"]
