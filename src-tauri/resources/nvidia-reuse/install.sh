#!/bin/sh
set -eu
cd "$1"
export UV_CACHE_DIR="$PWD/cache"
export HF_HUB_OFFLINE=1
export HF_HOME="$PWD/hf-cache"
export TORCH_HOME="$PWD/torch-cache"
export TRITON_CACHE_DIR="$PWD/triton-cache"

echo "Downloading runtime installer"
curl -fsSL --connect-timeout 20 --max-time 180 -o uv.tar.gz https://github.com/astral-sh/uv/releases/download/0.13.6/uv-x86_64-unknown-linux-gnu.tar.gz
echo 'f29c3e94ffabd7e6f3ba521dba169a9b7ec12100bf3257087f0f24f6123f3645  uv.tar.gz' | sha256sum -c - >/dev/null
tar -xzf uv.tar.gz --strip-components=1 uv-x86_64-unknown-linux-gnu/uv
rm uv.tar.gz

echo "Installing Python runtime"
curl -fsSL --connect-timeout 20 --max-time 180 -o python.tar.gz 'https://releases.astral.sh/github/python-build-standalone/releases/download/20261026/cpython-3.12.16%2B20261026-x86_64-unknown-linux-gnu-install_only_stripped.tar.gz'
echo 'ce4f6d0a6cfdb3bed6e44c2f9affd85943c76a42e25ad3a259002f31d6fb8a20  python.tar.gz' | sha256sum -c - >/dev/null
mkdir -p python
tar -xzf python.tar.gz --strip-components=1 -C python
rm python.tar.gz
./uv venv --clear --python "$PWD/python/bin/python3.12" .venv
echo "Downloading and installing GPU dependencies (several GB)"
./uv pip sync --python .venv/bin/python --require-hashes --index-strategy unsafe-best-match --index-url https://pypi.org/simple --extra-index-url https://download.pytorch.org/whl/cu128 --extra-index-url https://wheels.astral.sh/simple/cu128 requirements.lock >/dev/null 2>&1

echo "Downloading and verifying NVIDIA model"
base=https://huggingface.co/nvidia/NVIDIA-NemotronLabs-RE-USE/resolve/31f82f2efad1095034492f236e8dac9239ba713b
while read -r hash path; do
  mkdir -p "model/$(dirname "$path")"
  curl -fsSL --connect-timeout 20 --max-time 300 "$base/$path" -o "model/$path"
  echo "$hash  model/$path" | sha256sum -c - >/dev/null
done <<'ASSETS'
e69986483774f01893d2727503addc49ef201d19e6e1a266b8d4e971e28c478d config.json
a3d882953f026d3e2fdd57842d9c522c8320d4c0388af2278bcf4d438d255a2c LICENSE
06badaf4029b297f97f7fe48d62990b48d5482608210df577354e0d4e0ed70e1 model.safetensors
02bd1d478f36a9b38dce7d44368805298f42798ea239304580f1ccffafe766a0 models/codec_module_time_d4.py
beebaf2d9ab7b83c101f46246ae7a7a8a2d2f57b6683025d3712c64c1b7d6817 models/generator_SEMamba_time_d4.py
97397af2d7e0f5e6530aa8c4a45b4f0f0030b4b56d8c9922a9b0bc25f8b5962e models/mamba_block2_SEMamba.py
b82383c1ec7d7e9fe9bca1f93b3c0afe9609e191c38efc16f705e7145198d835 models/stfts.py
ASSETS
echo "Testing GPU and model"
timeout 120 .venv/bin/python worker.py --probe
echo ready > ready
echo "Ready"
