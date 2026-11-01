#!/bin/sh
set -eu

# Research experiment only. NVIDIA's model and reference code have their own
# terms; this does not change the license of Verenu or the OpenMDW model.
echo 'NVIDIA Real-time RE-USE is for research and development under the NVIDIA One-Way Noncommercial License (NSCLv1).'
echo 'Model card: https://huggingface.co/nvidia/Real-time_RE-USE'
test "$#" -eq 1
mkdir -p "$1"
cd "$1"
base=https://huggingface.co/nvidia/Real-time_RE-USE/resolve/4c0c186272ca853cd18d832fc5c28777b62f32d1
while read -r hash path; do
  mkdir -p "$(dirname "$path")"
  curl -fsSL --connect-timeout 20 --max-time 300 "$base/$path" -o "$path"
  echo "$hash  $path" | sha256sum -c - >/dev/null
done <<'ASSETS'
7c46c3279c4edbb3ae9b190905f90683f8db6ec38dea45da007e87b7d8b5621d config.json
6eac4e8cc721f99a22cde7eac3c9f8e2826ced5dc8a4234fd1977de1ec0d3ade model.safetensors
49dc7f57798c0f89f19c6f70ac4835b9e9efd43d02de499f02ed01450c7a6d4f online_inference.py
7bd63bb6f4a055fbd7c75e712e47c3c3923c15c969fb1ae6f3c2f2b6c15911a0 models/stfts.py
96b23a9efcb43de06a2b7f08d536e2f12ada940402f1b61a0505347e911470f2 models/streaming_codec_module_time_d1_input_ahead_sep_conv.py
838f8d50ebeedd97d1ad5dc3e43f53591a06d76c78b8e6552cf49f5b17a9405a models/streaming_generator_SEMamba_time_d1_random_layer_ahead_sep_conv.py
86026a8b987b33749c3ea3e71bfb83fe81c658ec51eb9392fb5fa814da7b08eb models/streaming_mamba_block2_SEMamba.py
f321af54469357756d335e96b49511b8d1db0c1782c1e555546ac51d5a414ee4 utils/util.py
715a32988ee8f05f35f78dcda8929ca2423e29ee598ad2ae13759563b589f67a README.md
ASSETS
