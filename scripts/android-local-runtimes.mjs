// Package native inference code with the APK. Models remain optional downloads.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const ORT_VERSION = '1.24.3'; // 1.24.x patch release; matches ort 2.0.0-rc.12's C API.
export const LLAMA_COMMIT = '6f4f53f2b7da54fcdbbecaaa734337c337ad6176'; // b9842
export const RUNTIME_ABIS = ['arm64-v8a', 'x86_64'];
export const LLAMA_ANDROID_BINARY = 'libverenu_llama_server.so';
const ORT_SHA256 = '67397e4a970e75617f765d2015ceaf911917e1d822276cfb5792744e8085cbce';
const LLAMA_SHA256 = 'c5200c3e3c98590a72c7751df17fb5b03ddceec90ed5914d7ebd960e821d3fa4';

export function cmakeArguments(source, build, ndk, abi) {
  if (!RUNTIME_ABIS.includes(abi)) throw new Error(`Unsupported local inference ABI: ${abi}`);
  return [
    '-G', 'Ninja',
    '-S', source, '-B', build,
    `-DCMAKE_TOOLCHAIN_FILE=${join(ndk, 'build', 'cmake', 'android.toolchain.cmake')}`,
    `-DANDROID_ABI=${abi}`, '-DANDROID_PLATFORM=android-28',
    '-DANDROID_STL=c++_shared', '-DCMAKE_BUILD_TYPE=Release',
    '-DANDROID_SUPPORT_FLEXIBLE_PAGE_SIZES=ON',
    // A single PIE executable avoids missing/versioned llama/ggml shared libs.
    '-DBUILD_SHARED_LIBS=OFF', '-DGGML_BACKEND_DL=OFF', '-DGGML_NATIVE=OFF',
    '-DGGML_OPENMP=OFF', '-DGGML_LLAMAFILE=OFF', '-DGGML_VULKAN=OFF',
    '-DLLAMA_CURL=OFF', '-DLLAMA_OPENSSL=OFF', '-DLLAMA_BUILD_TESTS=OFF',
    '-DLLAMA_BUILD_EXAMPLES=OFF', '-DLLAMA_BUILD_SERVER=ON',
    '-DMTMD_VIDEO=OFF', // Dictation cleanup never invokes ffmpeg/posix_spawn.
    '-DCMAKE_EXE_LINKER_FLAGS=-Wl,-z,max-page-size=16384 -Wl,-z,common-page-size=16384',
  ];
}

function ndkRoot() {
  for (const explicit of [process.env.ANDROID_NDK_HOME, process.env.ANDROID_NDK_ROOT]) {
    if (explicit && existsSync(join(explicit, 'build', 'cmake', 'android.toolchain.cmake'))) return explicit;
  }
  for (const sdk of [process.env.ANDROID_HOME, process.env.ANDROID_SDK_ROOT]) {
    if (!sdk || !existsSync(join(sdk, 'ndk'))) continue;
    const versions = readdirSync(join(sdk, 'ndk')).sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
    for (const version of versions) {
      const root = join(sdk, 'ndk', version);
      if (existsSync(join(root, 'build', 'cmake', 'android.toolchain.cmake'))) return root;
    }
  }
  throw new Error('Android local inference needs the NDK. Set ANDROID_HOME or ANDROID_NDK_HOME.');
}

async function verifiedArchive(url, path, expected) {
  if (!existsSync(path)) {
    const response = await fetch(url);
    if (!response.ok) throw new Error(`Native runtime download failed: HTTP ${response.status}`);
    writeFileSync(path, new Uint8Array(await response.arrayBuffer()));
  }
  const actual = createHash('sha256').update(readFileSync(path)).digest('hex');
  if (actual !== expected) throw new Error(`Native runtime checksum mismatch: ${path}`);
}

export async function syncAndroidLocalRuntimes(root = projectRoot) {
  const ndk = ndkRoot();
  const prebuilt = join(ndk, 'toolchains', 'llvm', 'prebuilt');
  const stripName = process.platform === 'win32' ? 'llvm-strip.exe' : 'llvm-strip';
  const strip = readdirSync(prebuilt).map(host => join(prebuilt, host, 'bin', stripName)).find(existsSync);
  if (!strip) throw new Error('Android local inference needs the NDK llvm-strip tool.');
  const cache = join(root, 'src-tauri', 'target', 'android-local-runtimes');
  const jni = join(root, 'src-tauri', 'gen', 'android', 'app', 'src', 'main', 'jniLibs');
  mkdirSync(cache, { recursive: true });
  const ortArchive = join(cache, `onnxruntime-android-${ORT_VERSION}.aar`);
  const llamaArchive = join(cache, `llama-${LLAMA_COMMIT}.tar.gz`);
  await verifiedArchive(`https://repo.maven.apache.org/maven2/com/microsoft/onnxruntime/onnxruntime-android/${ORT_VERSION}/onnxruntime-android-${ORT_VERSION}.aar`, ortArchive, ORT_SHA256);
  await verifiedArchive(`https://codeload.github.com/ggml-org/llama.cpp/tar.gz/${LLAMA_COMMIT}`, llamaArchive, LLAMA_SHA256);

  const ortExtract = join(cache, `ort-${ORT_VERSION}`);
  mkdirSync(ortExtract, { recursive: true });
  const jar = process.env.JAVA_HOME ? join(process.env.JAVA_HOME, 'bin', process.platform === 'win32' ? 'jar.exe' : 'jar') : 'jar';
  execFileSync(jar, ['xf', ortArchive, ...RUNTIME_ABIS.map(abi => `jni/${abi}/libonnxruntime.so`)], { cwd: ortExtract });
  const source = join(cache, `llama.cpp-${LLAMA_COMMIT}`);
  if (!existsSync(join(source, 'CMakeLists.txt'))) {
    execFileSync('cmake', ['-E', 'tar', 'xzf', llamaArchive], { cwd: cache });
  }
  for (const abi of RUNTIME_ABIS) {
    const build = join(cache, `build-ninja-${abi}`);
    const signature = JSON.stringify({ args: cmakeArguments(source, build, ndk, abi), ndk: readFileSync(join(ndk, 'source.properties'), 'utf8') });
    const stamp = join(build, 'verenu-runtime.json');
    const binary = join(build, 'bin', 'llama-server');
    if (!existsSync(binary) || !existsSync(stamp) || readFileSync(stamp, 'utf8') !== signature) {
      execFileSync('cmake', cmakeArguments(source, build, ndk, abi), { stdio: 'inherit' });
      execFileSync('cmake', ['--build', build, '--target', 'llama-server', '--parallel', process.env.CMAKE_BUILD_PARALLEL_LEVEL || '4'], { stdio: 'inherit' });
      writeFileSync(stamp, signature);
    }
    const dest = join(jni, abi);
    mkdirSync(dest, { recursive: true });
    copyFileSync(join(ortExtract, 'jni', abi, 'libonnxruntime.so'), join(dest, 'libonnxruntime.so'));
    // Android extracts lib*.so from the APK to its executable nativeLibraryDir.
    // It cannot execute binaries downloaded into the app's writable data dir.
    const packagedBinary = join(dest, LLAMA_ANDROID_BINARY);
    copyFileSync(binary, packagedBinary);
    // Keep symbols in the build cache for diagnosis, not in every phone APK.
    execFileSync(strip, ['--strip-unneeded', packagedBinary]);
  }
  const licenses = join(root, 'src-tauri', 'gen', 'android', 'app', 'src', 'main', 'assets', 'inference-licenses');
  mkdirSync(licenses, { recursive: true });
  copyFileSync(join(source, 'LICENSE'), join(licenses, 'llama.cpp-LICENSE'));
  const ortLicense = await fetch(`https://raw.githubusercontent.com/microsoft/onnxruntime/v${ORT_VERSION}/LICENSE`);
  if (!ortLicense.ok) throw new Error('ONNX Runtime license download failed');
  writeFileSync(join(licenses, 'onnxruntime-LICENSE'), await ortLicense.text());
  console.log('android-sync: bundled ONNX Runtime and llama.cpp for ARM64 and x86_64');
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await syncAndroidLocalRuntimes();
}
