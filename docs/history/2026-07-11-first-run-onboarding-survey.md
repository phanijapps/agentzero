# First-run onboarding survey

> Discipline: applied (practitioner-pattern survey)

## Question

What must a non-technical, first-time zbot user do to reach a safe first
successful conversation, and what should the product automate or defer?

## Current first-run path

The existing wizard has six sequential steps: agent identity, provider,
skills, MCP servers, agent/model configuration, and review/launch. It requires
a verified provider before advancing, but allows skills and MCP setup to be
skipped. Source: `apps/ui/src/features/setup/SetupWizard.tsx`.

Provider setup currently presents ten presets and asks the user to enter a raw
API key. It creates the provider, tests it, retains it only after success, and
marks the verified provider as the default candidate. Source:
`apps/ui/src/features/setup/steps/ProvidersStep.tsx`.

Ollama is labelled as a no-key option but assumes a reachable local service at
`localhost:11434`; the wizard does not install Ollama, diagnose its absence, or
download a model. Source: `apps/ui/src/features/settings/providerPresets.ts`.

The setup guard currently treats either `setupComplete` **or any provider** as
enough to bypass setup. This is inconsistent with the six-step wizard: a user
can have a provider but no selected model/default agent configuration. Source:
`apps/ui/src/features/setup/SetupGuard.tsx`.

Provider API keys are serialized to `config/providers.json` by the current
provider service. Source: `gateway/gateway-services/src/providers.rs`.

## Findings

### 1. The first-run critical path should be one choice, one credential, one proof

**[high]** A successful first message only needs a usable model route. The
current identity, skills, MCP, specialist-agent, and advanced token-routing
choices can be deferred because the wizard itself makes skills and MCP
optional, while provider verification is already the gating action. Source:
`SetupWizard.tsx`; [OpenAI quickstart](https://platform.openai.com/docs/quickstart/make-your-first-api-request);
[Ollama quickstart](https://docs.ollama.com/quickstart).

**Recommendation:** first-run should ask only:

1. “How would you like zbot to run?” — cloud provider or local Ollama.
2. “Choose a model” — one recommended default and an “other models” affordance.
3. “Connect” — API key entry or local-service detection.
4. A real test message, followed by the chat screen.

### 2. Advanced configuration belongs after first success

**[moderate]** Provider-specific fields, multiple providers, token limits,
skills, MCP servers, agent overrides, memory tuning, and automation settings
are advanced decisions. Progressive disclosure is appropriate because it keeps
complexity hidden until a user has demonstrated a need. Source:
`SetupWizard.tsx`; [Adjust wizard/progressive-disclosure guidance](https://atlas.adeven.com/docs/patterns/forms);
[progressive-disclosure research](https://arxiv.org/abs/1811.06426).

**Recommendation:** place these in a post-launch “Make zbot yours” checklist,
not in the blocking setup path. “Configure tools” must be opt-in, with a clear
statement of what each tool can access.

### 3. Local and cloud routes need different recovery experiences

**[high]** A cloud route needs account/key guidance and a test; a local Ollama
route needs installation/service detection and model download readiness. They
cannot share the same error message. Source: current zbot preset configuration;
[OpenAI quickstart](https://platform.openai.com/docs/quickstart/make-your-first-api-request);
[Ollama quickstart](https://docs.ollama.com/quickstart).

**Recommendation:** offer two primary cards:

- **Use a cloud model** — choose provider, open the provider’s key page, paste
  key, validate, choose recommended model.
- **Run locally with Ollama** — detect service, explain/download installer if
  absent, let the user choose a small recommended local model, pull it, then
  run a local test.

### 4. API keys must be handled as secrets, not ordinary configuration

**[high]** API keys can incur cost and permit access; provider guidance says to
keep them out of client code/repositories and use protected secret storage.
The present `providers.json` persistence is a security and comfort gap for a
desktop first-run flow. Source: `gateway/gateway-services/src/providers.rs`;
[OpenAI API authentication guidance](https://platform.openai.com/docs/api-reference/backward-compatibility?lang=ruby);
[Google API-key guidance](https://docs.cloud.google.com/docs/authentication/api-keys-best-practices);
[Microsoft secret-storage guidance](https://learn.microsoft.com/en-us/azure/well-architected/security/application-secrets).

**Recommendation:** store the key in an OS credential vault/keychain where
available; persist only a provider ID and secret reference in zbot config.
Show the provider, expected billing/privacy implication, and a “you can remove
this later” action before saving. Never echo a key after entry.

### 5. Privacy and comfort defaults must be explicit

**[moderate]** Cloud inference sends prompts to the selected provider; local
inference has different privacy, hardware, and availability trade-offs. OpenAI
documents provider-side data controls and retention options, illustrating why a
generic “cloud provider” label is insufficient. Source:
[OpenAI data controls](https://platform.openai.com/docs/models/default-usage-policies-by-endpoint);
[Ollama quickstart](https://docs.ollama.com/quickstart);
current zbot provider presets.

**Recommendation:** before connecting, state “messages are sent to <provider>”
for cloud routes and “messages stay on this device while Ollama is local” for
the local route. Default tools, filesystem access, MCP servers, automation, and
background memory enrichment to off until the user enables them later.

## Minimum first-run contract

Given a newly installed zbot, when a user completes setup, they can send and
receive one verified chat response without editing JSON/YAML, using a terminal,
knowing Rust/Node/React, selecting token limits, configuring agents, skills, or
MCP servers.

The setup outcome must persist:

- one verified default provider;
- one selected default model;
- a secure key reference for cloud providers, or a verified local endpoint and
  selected installed model for local providers;
- an explicit privacy route; and
- safe comfort defaults with advanced capabilities disabled.

## Known unknowns

- Which providers/models zbot should feature first depends on product cost,
  region, supported capabilities, and support burden; this needs a deliberate
  product decision rather than inference from the current preset list.
- The supported desktop platforms and available OS secret-vault abstraction are
  not established by this survey.
- Local model hardware detection, download size, and offline-install policy
  require a separate feasibility investigation.

## Sources

- zbot setup implementation: `apps/ui/src/features/setup/SetupWizard.tsx`,
  `ProvidersStep.tsx`, `SetupGuard.tsx`, and provider service sources cited
  above.
- [OpenAI quickstart](https://platform.openai.com/docs/quickstart/make-your-first-api-request)
- [Ollama quickstart](https://docs.ollama.com/quickstart)
- [OpenAI API authentication guidance](https://platform.openai.com/docs/api-reference/backward-compatibility?lang=ruby)
- [Google API-key best practices](https://docs.cloud.google.com/docs/authentication/api-keys-best-practices)
- [Microsoft secret-management guidance](https://learn.microsoft.com/en-us/azure/well-architected/security/application-secrets)
