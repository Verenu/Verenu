// Adapted from FluidAudio VocabularyBoostingSession.swift, Apache-2.0.
// Changes: explicit private tokenizer directory, no content logging, explicit
// English gate in the caller, conservative punctuation/alignment fallback.
import Foundation
import FluidAudio

struct AcousticVocabulary {
    let vocabulary: CustomVocabularyContext
    let spotter: CtcKeywordSpotter
    let rescorer: VocabularyRescorer
    let thresholds: ContextBiasingConstants.VocabSizeConfig

    init(directory: URL, terms: [String]) async throws {
        let tokenizer = try await CtcTokenizer.load(from: directory)
        vocabulary = CustomVocabularyContext(terms: terms.filter { $0.count >= 3 }.prefix(100).compactMap {
            let tokens = tokenizer.encode($0)
            return tokens.isEmpty ? nil : CustomVocabularyTerm(text: $0, ctcTokenIds: tokens)
        })
        let models = try await CtcModels.loadDirect(from: directory, variant: .ctc110m)
        spotter = CtcKeywordSpotter(models: models, blankId: models.vocabulary.count)
        thresholds = ContextBiasingConstants.rescorerConfig(forVocabSize: vocabulary.terms.count)
        rescorer = try await VocabularyRescorer.create(spotter: spotter, vocabulary: vocabulary,
            config: VocabularyBoostingSession.itnDefaultConfig, ctcModelDirectory: directory)
    }

    func rescore(text: String, timings: [TokenTiming], samples: [Float]) async throws -> String? {
        guard !vocabulary.terms.isEmpty else { return nil }
        let spotted = try await spotter.spotKeywordsWithLogProbs(audioSamples: samples,
            customVocabulary: vocabulary, minScore: nil)
        guard !spotted.logProbs.isEmpty else { return nil }
        let result = rescorer.ctcTokenRescore(transcript: text, tokenTimings: timings,
            logProbs: spotted.logProbs, frameDuration: spotted.frameDuration,
            cbw: thresholds.cbw, marginSeconds: 0.5,
            minSimilarity: max(thresholds.minSimilarity, vocabulary.minSimilarity))
        guard result.wasModified else { return nil }
        // Apply only acoustically accepted, unique spans to the primary text.
        // Preserve punctuation verbatim; ambiguous alignment retains primary.
        var output = text
        for replacement in result.replacements where replacement.shouldReplace {
            guard let word = replacement.replacementWord,
                  let range = output.range(of: replacement.originalWord, options: [.caseInsensitive]),
                  output.range(of: replacement.originalWord, options: [.caseInsensitive],
                    range: range.upperBound..<output.endIndex) == nil else { return nil }
            let before = range.lowerBound == output.startIndex ? nil : output[output.index(before: range.lowerBound)]
            let after = range.upperBound == output.endIndex ? nil : output[range.upperBound]
            guard before?.isLetter != true, before?.isNumber != true,
                  after?.isLetter != true, after?.isNumber != true else { return nil }
            output.replaceSubrange(range, with: word)
        }
        return output == text ? nil : output
    }
}
