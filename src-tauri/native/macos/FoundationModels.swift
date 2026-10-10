import Foundation
import FoundationModels

// C ABI owns only request IDs and copied UTF-8 strings. No Swift object or
// borrowed Rust buffer survives a call. A dropped Rust future cancels its task.
private final class Requests: @unchecked Sendable {
    static let shared = Requests()
    let lock = NSLock()
    var next: UInt64 = 0
    var tasks: [UInt64: Task<Void, Never>] = [:]
    var results: [UInt64: String] = [:]
}

@_cdecl("verenu_fm_availability")
public func availability() -> Int32 {
    guard #available(macOS 26.0, *) else { return 1 }
    switch SystemLanguageModel.default.availability {
    case .available: return 0
    case .unavailable(.deviceNotEligible): return 2
    case .unavailable(.appleIntelligenceNotEnabled): return 3
    case .unavailable(.modelNotReady): return 4
    case .unavailable: return 5
    }
}

private func complete(_ id: UInt64, _ value: String) {
    let requests = Requests.shared
    requests.lock.lock()
    defer { requests.lock.unlock() }
    // Cancellation removes the ID before cancelling the task. A late result
    // must never recreate it or retain dictated text indefinitely.
    if requests.tasks[id] != nil { requests.results[id] = value }
}

@_cdecl("verenu_fm_start")
public func start(_ instructions: UnsafePointer<CChar>, _ input: UnsafePointer<CChar>, _ tokens: Int32) -> UInt64 {
    let instructions = String(cString: instructions)
    let input = String(cString: input)
    let requests = Requests.shared
    requests.lock.lock()
    defer { requests.lock.unlock() }
    requests.next += 1
    let id = requests.next
    requests.tasks[id] = Task {
        guard #available(macOS 26.0, *), availability() == 0 else {
            complete(id, "E:unavailable")
            return
        }
        do {
            try Task.checkCancellation()
            // macOS 26.4 exposes the real tokenizer. Reserve space for output
            // and framing; never truncate a request to fit the model window.
            #if compiler(>=6.3)
            if #available(macOS 26.4, *) {
                let model = SystemLanguageModel.default
                let instructionCount = try await model.tokenCount(for: Instructions(instructions))
                let inputCount = try await model.tokenCount(for: input)
                if instructionCount + inputCount + Int(tokens) + 256 > 4096 {
                    complete(id, "E:context-overflow")
                    return
                }
            }
            #endif
            let session = LanguageModelSession(model: .default, tools: [], instructions: instructions)
            let response = try await session.respond(to: input, options: GenerationOptions(
                temperature: 0.2, maximumResponseTokens: Int(tokens)))
            try Task.checkCancellation()
            complete(id, "O:" + response.content)
        } catch is CancellationError {
            complete(id, "E:cancelled")
        } catch LanguageModelSession.GenerationError.exceededContextWindowSize {
            complete(id, "E:context-overflow")
        } catch LanguageModelSession.GenerationError.guardrailViolation {
            complete(id, "E:refusal")
        } catch LanguageModelSession.GenerationError.refusal {
            complete(id, "E:refusal")
        } catch LanguageModelSession.GenerationError.unsupportedLanguageOrLocale {
            complete(id, "E:unsupported-language")
        } catch LanguageModelSession.GenerationError.assetsUnavailable {
            complete(id, "E:model-not-ready")
        } catch {
            // Framework diagnostics may contain private input. Return a fixed
            // error code rather than an error description across IPC/logs.
            complete(id, "E:generation-failed")
        }
    }
    return id
}

@_cdecl("verenu_fm_poll")
public func poll(_ id: UInt64) -> UnsafeMutablePointer<CChar>? {
    let requests = Requests.shared
    requests.lock.lock()
    defer { requests.lock.unlock() }
    guard let result = requests.results.removeValue(forKey: id) else { return nil }
    requests.tasks.removeValue(forKey: id)
    return strdup(result)
}

@_cdecl("verenu_fm_cancel")
public func cancel(_ id: UInt64) {
    let requests = Requests.shared
    requests.lock.lock()
    let task = requests.tasks.removeValue(forKey: id)
    requests.results.removeValue(forKey: id)
    requests.lock.unlock()
    task?.cancel()
}

@_cdecl("verenu_fm_free")
public func freeResult(_ value: UnsafeMutablePointer<CChar>) { free(value) }
