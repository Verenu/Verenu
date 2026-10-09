#import <Foundation/Foundation.h>
#import <Speech/Speech.h>
#import <AVFoundation/AVFoundation.h>

long verenu_speech_os_major(void) { return NSProcessInfo.processInfo.operatingSystemVersion.majorVersion; }

static NSLocale *verenu_speech_locale(NSString *requested, NSLocale *current, NSSet<NSLocale *> *supported, BOOL (^eligible)(NSLocale *)) {
    NSString *identifier = [NSLocale canonicalLocaleIdentifierFromString:requested.length ? requested : current.localeIdentifier];
    NSDictionary *components = [NSLocale componentsFromLocaleIdentifier:identifier];
    NSString *language = components[NSLocaleLanguageCode];
    NSString *script = components[NSLocaleScriptCode];
    NSArray<NSLocale *> *ordered = [supported.allObjects sortedArrayUsingComparator:^NSComparisonResult(NSLocale *a, NSLocale *b) {
        return [a.localeIdentifier compare:b.localeIdentifier];
    }];
    NSLocale *fallback = nil;
    NSLocale *preferred = nil;
    for (NSLocale *locale in ordered) {
        NSString *candidate = [NSLocale canonicalLocaleIdentifierFromString:locale.localeIdentifier];
        NSDictionary *parts = [NSLocale componentsFromLocaleIdentifier:candidate];
        if (![parts[NSLocaleLanguageCode] isEqualToString:language]) continue;
        if (script.length && ![parts[NSLocaleScriptCode] isEqualToString:script]) continue;
        if (eligible && !eligible(locale)) continue;
        if ([candidate isEqualToString:identifier]) return locale;
        if (!fallback) fallback = locale;
        if ([candidate isEqualToString:[NSLocale canonicalLocaleIdentifierFromString:current.localeIdentifier]]) preferred = locale;
    }
    return preferred ?: fallback;
}

// Owned by one Rust blocking worker. Callbacks only update request-local state.
// Neither the transcript nor vocabulary is written to native diagnostics.
char *verenu_speech_transcribe(const float *samples, size_t count,
                             const char *language, const char *vocabularyJSON,
                             bool (*cancelled)(const void *), const void *cancelState,
                             int *errorCode) {
    @autoreleasepool {
        *errorCode = 0;
        if (!count) { *errorCode = 1; return NULL; }
        dispatch_semaphore_t permission = dispatch_semaphore_create(0);
        if ([SFSpeechRecognizer authorizationStatus] == SFSpeechRecognizerAuthorizationStatusNotDetermined) {
            dispatch_async(dispatch_get_main_queue(), ^{
                [SFSpeechRecognizer requestAuthorization:^(SFSpeechRecognizerAuthorizationStatus status) {
                    dispatch_semaphore_signal(permission);
                }];
            });
            NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:120];
            while (dispatch_semaphore_wait(permission, dispatch_time(DISPATCH_TIME_NOW, 50000000))) {
                if (cancelled(cancelState)) { *errorCode = 2; return NULL; }
                if (deadline.timeIntervalSinceNow <= 0) { *errorCode = 3; return NULL; }
            }
        }
        if ([SFSpeechRecognizer authorizationStatus] != SFSpeechRecognizerAuthorizationStatusAuthorized) {
            *errorCode = 4; return NULL;
        }
        NSString *localeName = [NSString stringWithUTF8String:language];
        NSMutableDictionary<NSString *, SFSpeechRecognizer *> *recognizers = [NSMutableDictionary dictionary];
        NSLocale *locale = verenu_speech_locale(localeName, [NSLocale currentLocale], [SFSpeechRecognizer supportedLocales], ^BOOL(NSLocale *candidateLocale) {
            SFSpeechRecognizer *candidate = [[SFSpeechRecognizer alloc] initWithLocale:candidateLocale];
            if (!candidate.available || !candidate.supportsOnDeviceRecognition) return NO;
            recognizers[candidateLocale.localeIdentifier] = candidate;
            return YES;
        });
        if (!locale) { *errorCode = 5; return NULL; }
        SFSpeechRecognizer *recognizer = recognizers[locale.localeIdentifier];
        if (!recognizer || !recognizer.available || !recognizer.supportsOnDeviceRecognition) {
            *errorCode = 5; return NULL;
        }
        recognizer.queue = [[NSOperationQueue alloc] init];
        recognizer.queue.maxConcurrentOperationCount = 1;
        SFSpeechAudioBufferRecognitionRequest *request = [[SFSpeechAudioBufferRecognitionRequest alloc] init];
        request.requiresOnDeviceRecognition = YES;
        request.shouldReportPartialResults = NO;
        request.taskHint = SFSpeechRecognitionTaskHintDictation;
        if (@available(macOS 13.0, *)) { request.addsPunctuation = YES; }
        NSData *json = [[NSString stringWithUTF8String:vocabularyJSON] dataUsingEncoding:NSUTF8StringEncoding];
        NSArray *terms = [NSJSONSerialization JSONObjectWithData:json options:0 error:nil];
        request.contextualStrings = terms ?: @[];
        AVAudioFormat *format = [[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatFloat32
                                                             sampleRate:16000 channels:1 interleaved:NO];
        if (count > UINT32_MAX) { *errorCode = 1; return NULL; }
        AVAudioPCMBuffer *buffer = [[AVAudioPCMBuffer alloc] initWithPCMFormat:format frameCapacity:(AVAudioFrameCount)count];
        if (!buffer) { *errorCode = 1; return NULL; }
        buffer.frameLength = (AVAudioFrameCount)count;
        memcpy(buffer.floatChannelData[0], samples, count * sizeof(float));
        NSObject *lock = [[NSObject alloc] init];
        __block BOOL finished = NO;
        __block NSString *text = nil;
        __block int resultError = 0;
        dispatch_semaphore_t completion = dispatch_semaphore_create(0);
        SFSpeechRecognitionTask *task = [recognizer recognitionTaskWithRequest:request resultHandler:^(SFSpeechRecognitionResult *result, NSError *error) {
            @synchronized(lock) {
                if (finished) return;
                if (result.isFinal) { text = result.bestTranscription.formattedString; }
                else if (error) { resultError = 6; }
                else { return; }
                finished = YES;
                dispatch_semaphore_signal(completion);
            }
        }];
        [request appendAudioPCMBuffer:buffer];
        [request endAudio];
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:120];
        while (dispatch_semaphore_wait(completion, dispatch_time(DISPATCH_TIME_NOW, 50000000))) {
            if (cancelled(cancelState) || deadline.timeIntervalSinceNow <= 0) {
                @synchronized(lock) {
                    if (!finished) { finished = YES; resultError = cancelled(cancelState) ? 2 : 3; }
                }
                break;
            }
        }
        [task cancel];
        @synchronized(lock) {
            *errorCode = resultError;
            return text ? strdup(text.UTF8String) : NULL;
        }
    }
}
