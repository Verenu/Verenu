# Vocabulary

Vocabulary tells Verenu about words that transcription models often miss, such as names, brands, product names, and technical terms.

Vocabulary belongs to a context. Add it to **Everywhere** when it should apply to all dictation, or add it to a specific context when it only belongs in certain apps or websites.

## Add a vocabulary entry

1. Open **Contexts** and select the context where the term belongs.
2. Open the **Vocabulary** tab.
3. Choose **+ Term**.
4. Enter the exact **Term** you want Verenu to use.
5. Optionally enter one or more values in **Often mistranscribed as**. Separate multiple mistakes with commas.
6. Save the term.

Terms and mistranscriptions can each be up to 120 characters.

The term field is the right choice when you want the cleanup model to recognize a word. The mistranscription field is useful when the transcription model repeatedly produces a specific wrong spelling. Leave it empty when there is no consistent mistake to replace.

You can dictate into either field with the small microphone button beside it.

## How vocabulary is used

When a context is active, Verenu uses its vocabulary while preparing and cleaning the dictation. Distinctive corrections can also be applied mechanically after cleanup. Common, ambiguous corrections are handled contextually so a learned correction does not rewrite every ordinary use of a word.

An entry can belong to several contexts. To reuse one, add it to another context rather than creating a duplicate. To remove it from the current context without deleting it, open the row menu and choose **Move to...**.

The canonical term can therefore be shared while its mistranscription mappings
remain context-specific. Adding a shared term to another context does not copy
AutoLearn mappings or evidence into that context; a mapping becomes effective
there only when it is explicitly entered, intentionally moved, or learned
there. **Everywhere** is the fallback context when no targeted context matches,
not a parent whose vocabulary is inherited by every targeted context.

Deleting the entry removes it from every context. Deleting a context moves its entries to Everywhere instead.

## Auto-learned vocabulary

When Auto-learn is enabled, Verenu watches the focused text field for corrections after a dictation. Repeated corrections can become vocabulary entries automatically.

- Distinctive terms, such as brand names and technical words, can be promoted after one high-confidence correction.
- Ordinary words need repeated corrections before Verenu promotes them.
- Evidence accumulates for 30 days, so occasional corrections can still teach a term.
- Brand capitalization, an isolated acronym such as `api` to `API` within a sentence, and split names such as `open ai` to `OpenAI` can be learned. Ordinary sentence capitalization, punctuation, and all-caps formatting are ignored.
- Auto-learned entries show an indicator and confidence information in the Vocabulary list.

Correct the inserted text in the same field within 60 seconds of dictation,
then pause for about a second before sending it or switching away. Single-word
dictations count too. Auto-learn reads only a stable correction in the original
field; switching fields does not teach it a new mapping.

The editor must expose editable text through the operating system's
accessibility API. Windows uses UI Automation, macOS requires Accessibility
permission, and Linux uses AT-SPI. Password fields are excluded. Custom editors
that do not expose text cannot be monitored. On Linux, `NO_AT_BRIDGE=1` disables
GTK accessibility even when the accessibility bus is enabled; start the editor
without that variable to make its corrections readable.

The short-lived candidate and pending-observation records stay on the device.
Once promoted, the persistent correction is stored with the originating
context, survives restart, and is included in context-aware backup/sync data.

Auto-learn monitors the text field after insertion. It does not send the monitoring data to a Verenu server.

## Vocabulary or snippet?

Use **Vocabulary** when the output should stay in the sentence but use the right spelling or term.

Use a [Snippet](SNIPPETS.md) when a spoken trigger should insert a saved phrase, address, template, or block of text.

## Legacy page

Older installations may still show the standalone Dictionary page. It is hidden by default and is not the main setup path. Turn on **Settings -> General -> Legacy pages** only when you need to maintain an older view of the same local vocabulary data.

## Related docs

<p align="center">
  <a href="CONTEXTS.md"><img alt="Contexts" src="https://img.shields.io/badge/Contexts-Guide-a3352b"></a>
  <a href="SNIPPETS.md"><img alt="Snippets" src="https://img.shields.io/badge/Snippets-Guide-c44632"></a>
  <a href="DATA_AND_PRIVACY.md"><img alt="Data and Privacy" src="https://img.shields.io/badge/Data-Privacy-5b554a"></a>
</p>
