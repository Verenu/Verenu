#include "../Speech.m"
#include <assert.h>
int main(void) {
    @autoreleasepool {
        NSLocale *us = [NSLocale localeWithLocaleIdentifier:@"en-US"];
        NSLocale *gb = [NSLocale localeWithLocaleIdentifier:@"en-GB"];
        NSSet *supported = [NSSet setWithObjects:us, gb, [NSLocale localeWithLocaleIdentifier:@"fr-FR"], nil];
        assert([verenu_speech_locale(@"en", gb, supported, nil) isEqual:gb]);
        assert([verenu_speech_locale(@"en-US", gb, supported, nil) isEqual:us]);
        assert([verenu_speech_locale(@"", gb, supported, nil) isEqual:gb]);
        assert(verenu_speech_locale(@"xx", gb, supported, nil) == nil);
        assert([verenu_speech_locale(@"en", gb, supported, ^BOOL(NSLocale *locale) { return [locale isEqual:us]; }) isEqual:us]);
        assert(verenu_speech_locale(@"en", gb, supported, ^BOOL(NSLocale *locale) { return NO; }) == nil);
        NSLocale *actual = verenu_speech_locale(@"en", us, [SFSpeechRecognizer supportedLocales], nil);
        assert(actual != nil);
        assert([[NSLocale componentsFromLocaleIdentifier:actual.localeIdentifier][NSLocaleLanguageCode] isEqualToString:@"en"]);
        assert([[SFSpeechRecognizer supportedLocales] containsObject:actual]);
        puts("PASS: explicit language, regional preference, auto, unsupported language and actual supported English locale selection. No authorization or recognition requested.");
    }
    return 0;
}
