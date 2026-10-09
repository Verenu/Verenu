// swift-tools-version: 6.2
import PackageDescription
let package = Package(
    name: "VerenuFluidSpeech", platforms: [.macOS(.v14)],
    products: [.executable(name: "VerenuFluidSpeech", targets: ["VerenuFluidSpeech"])],
    dependencies: [.package(url: "https://github.com/FluidInference/FluidAudio.git",
        revision: "0b1f46289fe27d95b5e66ad8be46e64f5ee02ae7", traits: [])],
    targets: [.executableTarget(name: "VerenuFluidSpeech",
        dependencies: [.product(name: "FluidAudio", package: "FluidAudio")])]
)
