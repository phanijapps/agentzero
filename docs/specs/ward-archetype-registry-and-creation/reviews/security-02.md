## Concerns

1. **[reason] Doctrine injection validation is still bypassable with normal
   filler words.** Scan a bounded lookahead after normalized reset verbs so
   phrases such as `ignore the previous instructions` and `override these
   earlier rules` are rejected.

2. **[reason] Role-label detection misses common Markdown wrappers.** Normalize
   list, blockquote, heading, emphasis, and code markers before checking
   system, developer, assistant, and user role labels.
