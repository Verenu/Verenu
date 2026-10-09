import Foundation
import FluidAudio

struct Request: Decodable {
    let operation: String
    let path: String?
    let model: String?
    let samples: [Float]?
    let language: String?
    let vocabulary: [String]?
    let boosterPath: String?
}
struct Response: Encodable {
    let text: String?
    let error: String?
    let boosting: String?
}

// One private pipe, one serialized manager. No network access during loading
// or inference. Exiting the process releases all CoreML model allocations.
@main struct Bridge {
    static func main() async {
        AppLogger.minimumLevel = .fault
        AppLogger.mirrorsToConsole = false
        ModelHub.offlineMode = true
        let manager = AsrManager()
        var version: AsrModelVersion = .v3
        var cachedVocabulary: AcousticVocabulary?
        var cachedTerms: [String] = []
        var cachedPath: String?
        while let line = readLine() {
            let response: Response
            do {
                let request = try JSONDecoder().decode(Request.self, from: Data(line.utf8))
                if request.operation == "load" {
                    cachedVocabulary = nil
                    cachedTerms = []
                    cachedPath = nil
                    if request.model == "fluid-english-booster" {
                        guard let path = request.path else { throw BridgeError.invalid }
                        let directory = URL(fileURLWithPath: path)
                        _ = try await CtcModels.loadDirect(from: directory, variant: .ctc110m)
                        _ = try await CtcTokenizer.load(from: directory)
                        let response = Response(text: nil, error: nil, boosting: nil)
                        let data = try JSONEncoder().encode(response)
                        FileHandle.standardOutput.write(data)
                        FileHandle.standardOutput.write(Data([10]))
                        continue
                    }
                    switch request.model {
                    case "fluid-parakeet-ultra": version = .ultra
                    case "fluid-parakeet-110m": version = .tdtCtc110m
                    case "fluid-parakeet-ja": version = .tdtJa
                    default: throw BridgeError.invalid
                    }
                    guard let path = request.path else { throw BridgeError.invalid }
                    try await manager.loadModels(AsrModels.loadLocal(from: URL(fileURLWithPath: path), version: version))
                    response = Response(text: nil, error: nil, boosting: nil)
                } else if request.operation == "transcribe" {
                    guard let samples = request.samples, samples.count <= 16_000 * 600,
                          samples.allSatisfy({ $0.isFinite }) else { throw BridgeError.invalid }
                    var state = TdtDecoderState.make(decoderLayers: await manager.decoderLayerCount)
                    let result = try await manager.transcribe(samples, decoderState: &state,
                        language: request.language.flatMap(Language.init(rawValue:)))
                    var text = result.text
                    var boosting = "skipped"
                    // Automatic language has no acoustic English guarantee. Only
                    // explicitly English dictation uses this English CTC model.
                    if request.language == "en", version != .tdtJa,
                       let path = request.boosterPath, let terms = request.vocabulary,
                       !terms.isEmpty, let timings = result.tokenTimings, !timings.isEmpty {
                        do {
                            if cachedTerms != terms || cachedPath != path || cachedVocabulary == nil {
                                cachedVocabulary = try await AcousticVocabulary(directory: URL(fileURLWithPath: path), terms: terms)
                                cachedTerms = terms
                                cachedPath = path
                            }
                            let session = cachedVocabulary!
                            if let corrected = try await session.rescore(text: text, timings: timings, samples: samples) {
                                text = corrected
                                boosting = "applied"
                            } else { boosting = "unchanged" }
                        } catch { boosting = "unavailable" }
                    }
                    else {
                        cachedVocabulary = nil
                        cachedTerms = []
                        cachedPath = nil
                    }
                    response = Response(text: text, error: nil, boosting: boosting)
                } else { throw BridgeError.invalid }
            } catch { response = Response(text: nil, error: "native_operation_failed", boosting: nil) }
            if let data = try? JSONEncoder().encode(response) {
                FileHandle.standardOutput.write(data)
                FileHandle.standardOutput.write(Data([10]))
            }
        }
    }
}
enum BridgeError: Error { case invalid }
