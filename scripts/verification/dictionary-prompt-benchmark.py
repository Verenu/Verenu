#!/usr/bin/env python3
"""Compare dictionary instructions using synthetic dictation and a saved Google key."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
CASES = [
    ('technical', 'Kubernetes', 'communities', 'Deploy the service to communities using kubectl.', 'Deploy the service to Kubernetes using kubectl.'),
    ('ordinary', 'Kubernetes', 'communities', 'The local communities organized a food drive.', 'The local communities organized a food drive.'),
    ('database', 'PostgreSQL', 'post grass', 'Run the migration on post grass before restarting the database.', 'Run the migration on PostgreSQL before restarting the database.'),
    ('literal', 'PostgreSQL', 'post grass', 'Please post grass clippings in the garden forum.', 'Please post grass clippings in the garden forum.'),
    ('app', 'Verenu', 'Verena', 'Open Verena and start dictation.', 'Open Verenu and start dictation.'),
    ('person', 'Verenu', 'Verena', 'Verena will join us for dinner tonight.', 'Verena will join us for dinner tonight.'),
    ('brand', 'Notion', 'notion', 'Add the project notes to notion in the workspace.', 'Add the project notes to Notion in the workspace.'),
    ('common', 'Notion', 'notion', 'I reject the notion that deadlines are optional.', 'I reject the notion that deadlines are optional.'),
]
VARIANTS = {
    'baseline': '',
    'strongly-supported': '; replace only when context strongly supports it',
    'cautious': '; cautious: retain the variant unless the context clearly refers to the preferred term',
    'favor': '; favor replacement when plausible',
    'literal-possible': '; variant may be a correct word or person; preserve it unless context unambiguously identifies the preferred term',
    'not-names': '; retain the variant when it fits the context, including as a person or ordinary word; change only for clear contextual evidence',
    'strict': '; variant is ambiguous. Preserve it unless other words make the preferred term unmistakable',
    'system-cautious': '; cautious',
    'system-names': '; cautious',
}


def normalize(text):
    # Terminal punctuation can vary; words, order, spelling, and case cannot.
    return re.sub(r'[.!?]+$', '', ' '.join(text.split()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--model', default='gemini-3.5-flash')
    parser.add_argument('--repeat', type=int, default=1, choices=range(1, 6))
    parser.add_argument('--variant', choices=VARIANTS, help='Run only one instruction')
    args = parser.parse_args()
    if not re.fullmatch(r'[A-Za-z0-9._-]+', args.model):
        parser.error('Invalid model name')
    key = os.environ.get('GOOGLE_API_KEY')
    if not key:
        try:
            key = subprocess.check_output(['secret-tool', 'lookup', 'service', 'com.verenu.app', 'username', 'api_key_google'], timeout=10, stderr=subprocess.DEVNULL).decode().strip()
        except (OSError, subprocess.SubprocessError):
            print('[ERROR] Google credential unavailable', file=sys.stderr)
            return 1
    source = (ROOT / 'src-tauri/src/api/prompts/cleanup_templates.rs').read_text()
    template = source.split('const DEFAULT_CLEANUP_TEMPLATE: &str = r#"', 1)[1].split('"#;', 1)[0]
    prompt = template.replace('{{ cleanup_preset }}', 'Cleanup: Light. Remove fillers only when non-semantic, accidental repeats, and abandoned starts. Preserve meaningful uses. Fix punctuation and capitalization. Otherwise preserve words and order; do not paraphrase.\nTone: Casual. Preserve conversational voice and natural contractions. Use normal casing and punctuation.').replace('{{ formatting_rules }}', 'Keep dictated layout; do not create paragraphs, lists, or headings from content alone.').replace('{{ snippet_overrides }}', '').replace('{{ evidence }}', '')
    failures = 0
    for variant, instruction in VARIANTS.items():
        if args.variant and args.variant != variant:
            continue
        if not args.variant and variant not in ('baseline', 'favor', 'system-names'):
            continue
        for repeat in range(args.repeat):
            for case_id, term, mistranscription, transcript, expected in CASES:
                evidence = f'\n\n<evidence>\nVocabulary evidence (not a replacement list):\n- preferred: "{term}"; possible STT variants: "{mistranscription}"{instruction}\n</evidence>'
                generation = {'maxOutputTokens': max(96, min(768, len(transcript.split()) * 2 + 32))}
                if args.model.startswith('gemini-3'):
                    generation['thinkingConfig'] = {'thinkingLevel': 'minimal'}
                else:
                    generation.update({'temperature': 0, 'thinkingConfig': {'thinkingBudget': 0}})
                policy = '\nFor cautious vocabulary entries, variants may be correct names or ordinary words. Preserve a variant unless the surrounding context clearly refers to the preferred term.' if variant == 'system-cautious' else ''
                if variant == 'system-names':
                    policy = '\nFor cautious vocabulary entries, preserve a variant used as a person\'s name or an ordinary word. Replace it only when context identifies the preferred term, such as using an app or database.'
                payload = {'system_instruction': {'parts': [{'text': prompt + policy + evidence}]}, 'contents': [{'role': 'user', 'parts': [{'text': f'<raw_dictation>\n{transcript}\n</raw_dictation>'}]}], 'generationConfig': generation}
                request = urllib.request.Request(f'https://generativelanguage.googleapis.com/v1beta/models/{args.model}:generateContent', data=json.dumps(payload).encode(), headers={'Content-Type': 'application/json', 'x-goog-api-key': key})
                try:
                    with urllib.request.urlopen(request, timeout=45) as response:
                        result = json.load(response)
                    output = ''.join(part.get('text', '') for part in result['candidates'][0]['content']['parts'])
                    finish = result['candidates'][0].get('finishReason')
                    passed = finish == 'STOP' and normalize(output) == normalize(expected)
                    print(json.dumps({'variant': variant, 'run': repeat + 1, 'case': case_id, 'pass': passed, 'finish': finish, 'output': output, 'input_tokens': result.get('usageMetadata', {}).get('promptTokenCount')}), flush=True)
                    failures += not passed
                except (urllib.error.URLError, TimeoutError, KeyError, IndexError, ValueError) as error:
                    print(json.dumps({'variant': variant, 'run': repeat + 1, 'case': case_id, 'error': type(error).__name__}), flush=True)
                    failures += 1
    return 1 if failures else 0


if __name__ == '__main__':
    sys.exit(main())
