ARG BASE_IMAGE=verenu-dev:local
FROM ${BASE_IMAGE}
RUN apt-get update && apt-get install -y --no-install-recommends \
    webkit2gtk-driver pulseaudio pulseaudio-utils espeak-ng ffmpeg python3 && \
    rm -rf /var/lib/apt/lists/*
