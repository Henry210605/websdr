# WebSDR – browserbasierter Empfänger für den RTL-SDR Blog V4
# Mehrstufiger Build: V4-Treiber, Frontend und Rust-Backend werden getrennt gebaut,
# das Laufzeit-Image enthält nur das Nötigste.

ARG RTLSDR_BLOG_VERSION=v1.3.6

# --- Stufe 1: librtlsdr mit V4-Unterstützung (rtl-sdr-blog) ---
FROM debian:bookworm-slim AS rtlbuild
ARG RTLSDR_BLOG_VERSION
RUN apt-get update && apt-get install -y --no-install-recommends \
      git cmake build-essential pkg-config libusb-1.0-0-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN git clone --depth 1 --branch ${RTLSDR_BLOG_VERSION} https://github.com/rtlsdrblog/rtl-sdr-blog /src/rtl-sdr-blog
WORKDIR /src/rtl-sdr-blog/build
RUN cmake .. -DCMAKE_INSTALL_PREFIX=/usr/local -DINSTALL_UDEV_RULES=OFF -DDETACH_KERNEL_DRIVER=ON \
 && make -j"$(nproc)" \
 && make install DESTDIR=/out

# --- Stufe 2: Frontend ---
FROM node:22-bookworm-slim AS webbuild
WORKDIR /web
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

# --- Stufe 3: Rust-Backend ---
FROM rust:1-bookworm AS rustbuild
WORKDIR /src
# Abhängigkeiten zuerst bauen, damit Codeänderungen den Cache nicht ungültig machen
COPY backend/Cargo.toml backend/Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src
COPY backend/src ./src
RUN touch src/main.rs && cargo build --release

# --- Stufe 4: Laufzeit ---
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends libusb-1.0-0 curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=rtlbuild /out/usr/local/ /usr/local/
RUN ldconfig
COPY --from=rustbuild /src/target/release/websdr /usr/local/bin/websdr
COPY --from=webbuild /web/dist /app/static
WORKDIR /app
ENV WEBSDR_LISTEN=0.0.0.0:8073 \
    WEBSDR_STATIC=/app/static \
    WEBSDR_DATA=/app/data \
    SDR_SOURCE=usb
VOLUME /app/data
EXPOSE 8073
HEALTHCHECK --interval=30s --timeout=3s CMD curl -fs http://localhost:8073/health || exit 1
CMD ["websdr"]
