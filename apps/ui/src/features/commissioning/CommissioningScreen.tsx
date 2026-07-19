import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import {
  ArrowRight,
  Bot,
  Check,
  Cloud,
  Code2,
  HeartHandshake,
  Loader2,
  LockKeyhole,
  RefreshCw,
  Search,
  ShieldCheck,
  Sparkles,
  Wrench,
} from "lucide-react";
import { getTransport } from "@/services/transport";
import type { CommissioningRequest, LocalDiagnosis } from "@/services/transport";

type Step = "focus" | "intelligence" | "memory" | "world";
type Focus = CommissioningRequest["primaryFocus"];
type Domain = CommissioningRequest["domains"][number];
type MemoryProfile = CommissioningRequest["memoryProfile"];
type CloudPreset = NonNullable<CommissioningRequest["provider"]["presetId"]>;

const STEPS: Array<{ id: Step; label: string }> = [
  { id: "focus", label: "Your focus" },
  { id: "intelligence", label: "Your intelligence" },
  { id: "memory", label: "Your memory" },
  { id: "world", label: "About you" },
];

const FOCUSES: Array<{ id: Focus; title: string; description: string; icon: typeof Sparkles; domains: Domain[] }> = [
  { id: "think_organize", title: "Think & organize", description: "Keep ideas, plans, and personal knowledge clear.", icon: Sparkles, domains: ["personal_knowledge", "planning"] },
  { id: "build_code", title: "Build & code", description: "Plan, build, and improve technical work.", icon: Code2, domains: ["software", "planning"] },
  { id: "research_learn", title: "Research & learn", description: "Investigate questions and retain useful context.", icon: Search, domains: ["learning", "writing"] },
  { id: "run_work", title: "Run my work", description: "Bring structure to recurring projects and decisions.", icon: Wrench, domains: ["planning", "writing"] },
];

const DOMAINS: Array<{ id: Domain; label: string }> = [
  { id: "personal_knowledge", label: "Personal knowledge" },
  { id: "software", label: "Software" },
  { id: "writing", label: "Writing" },
  { id: "learning", label: "Learning" },
  { id: "planning", label: "Planning" },
];

const INTERESTS = [
  "Technology & software",
  "Learning & ideas",
  "Writing & creativity",
  "Health & wellbeing",
  "Finance & markets",
  "Travel & culture",
  "Gaming & entertainment",
  "Home & life",
  "Science & nature",
];

const CLOUD_PROVIDERS: Record<CloudPreset, { name: string; models: string[] }> = {
  ollama_cloud: { name: "Ollama Cloud", models: ["glm-5.2:cloud"] },
  openai: { name: "OpenAI", models: ["gpt-4o", "gpt-4o-mini", "o4-mini", "gpt-4.1"] },
  deepseek: { name: "DeepSeek", models: ["deepseek-chat", "deepseek-reasoner"] },
  openrouter: { name: "OpenRouter", models: ["anthropic/claude-opus", "openai/gpt-4-turbo", "google/gemini-pro"] },
  "z-ai": { name: "Z.AI", models: ["glm-5.1", "glm-5", "glm-5-turbo", "glm-4.7", "glm-4.6", "glm-4.5"] },
  mistral: { name: "Mistral", models: ["mistral-large-latest", "mistral-small-latest", "codestral-latest"] },
};

interface CommissioningScreenProps {
  /** Allow an already commissioned user to intentionally rerun setup. */
  rerunSetup?: boolean;
}

export function CommissioningScreen({ rerunSetup = false }: CommissioningScreenProps) {
  const [step, setStep] = useState<Step>("focus");
  const [focus, setFocus] = useState<Focus | null>(null);
  const [domains, setDomains] = useState<Domain[]>([]);
  const [providerKind, setProviderKind] = useState<"cloud" | "local">("cloud");
  const [preset, setPreset] = useState<CloudPreset>("openai");
  const [model, setModel] = useState(CLOUD_PROVIDERS.openai.models[0]);
  const [apiKey, setApiKey] = useState("");
  const [localDiagnosis, setLocalDiagnosis] = useState<LocalDiagnosis | null>(null);
  const [isDiagnosing, setIsDiagnosing] = useState(false);
  const [displayName, setDisplayName] = useState("My Agent");
  const [userName, setUserName] = useState("");
  const [interests, setInterests] = useState<string[]>([]);
  const [hobbies, setHobbies] = useState("");
  const [dateOfBirth, setDateOfBirth] = useState("");
  const [profile, setProfile] = useState("");
  const [memoryProfile, setMemoryProfile] = useState<MemoryProfile | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [restartPending, setRestartPending] = useState(false);
  const [isCheckingActivation, setIsCheckingActivation] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const navigate = useNavigate();

  useEffect(() => {
    let mounted = true;
    void getTransport()
      .then((transport) => transport.getCommissioningStatus())
      .then((result) => {
        if (!mounted || !result.success || !result.data) return;
        if (result.data.state === "complete" && !result.data.restartRequired && !rerunSetup) {
          navigate("/", { replace: true });
        } else if (result.data.restartRequired) {
          setRestartPending(true);
        } else if (result.data.recoveryCode === "memory_profile_conflict") {
          setError("The Full Zbot memory files or settings changed before activation. Resolve the conflict, then submit commissioning again; z-Bot did not overwrite the existing configuration.");
        }
      })
      .catch(() => {
        // A transient status failure must not prevent a fresh user from using
        // the local commissioning wizard.
      });
    return () => {
      mounted = false;
    };
  }, [navigate, rerunSetup]);

  const currentIndex = STEPS.findIndex((item) => item.id === step);
  const focusDetails = focus ? FOCUSES.find((item) => item.id === focus) : undefined;
  const selectedModelOptions = providerKind === "cloud"
    ? CLOUD_PROVIDERS[preset].models
    : localDiagnosis?.models || [];
  const canContinue = useMemo(() => {
    if (step === "focus") return Boolean(focus) && domains.length > 0;
    if (step === "intelligence") {
      return providerKind === "local"
        ? localDiagnosis?.state === "ready" && Boolean(model)
        : Boolean(apiKey.trim()) && Boolean(model);
    }
    if (step === "memory") return memoryProfile !== null;
    if (step === "world") return Boolean(displayName.trim()) && Boolean(userName.trim()) && interests.length > 0;
    return false;
  }, [apiKey, displayName, domains.length, focus, interests.length, localDiagnosis?.state, memoryProfile, model, providerKind, step, userName]);

  const selectFocus = (nextFocus: Focus) => {
    const selection = FOCUSES.find((item) => item.id === nextFocus)!;
    setFocus(nextFocus);
    setDomains(selection.domains);
  };

  const toggleDomain = (domain: Domain) => {
    setDomains((current) => current.includes(domain)
      ? current.filter((item) => item !== domain)
      : [...current, domain]);
  };

  const toggleInterest = (interest: string) => {
    setInterests((current) => current.includes(interest)
      ? current.filter((item) => item !== interest)
      : [...current, interest]);
  };

  const choosePreset = (nextPreset: CloudPreset) => {
    setApiKey("");
    setPreset(nextPreset);
    setModel(CLOUD_PROVIDERS[nextPreset].models[0]);
    setError(null);
  };

  const diagnoseLocal = async () => {
    setIsDiagnosing(true);
    setError(null);
    try {
      const transport = await getTransport();
      const result = await transport.diagnoseLocalRuntime();
      if (!result.success || !result.data) {
        setError("We couldn’t check the local runtime. Try again.");
        return;
      }
      setLocalDiagnosis(result.data);
      if (result.data.models?.[0]) setModel(result.data.models[0]);
    } catch {
      setError("We couldn’t check the local runtime. Try again.");
    } finally {
      setIsDiagnosing(false);
    }
  };

  const next = () => {
    if (!canContinue) return;
    const nextStep = STEPS[currentIndex + 1];
    if (nextStep) setStep(nextStep.id);
  };

  const back = () => {
    const previous = STEPS[currentIndex - 1];
    if (previous) setStep(previous.id);
  };

  const complete = async () => {
    if (!focus || !memoryProfile || !canContinue) return;
    setIsSubmitting(true);
    setError(null);
    const request: CommissioningRequest = {
      displayName: displayName.trim(),
      profile: profile.trim() || undefined,
      userName: userName.trim(),
      interests,
      hobbies: splitList(hobbies),
      dateOfBirth: dateOfBirth || undefined,
      primaryFocus: focus,
      domains,
      memoryProfile,
      provider: providerKind === "cloud"
        ? { kind: "cloud", presetId: preset, model, apiKey: apiKey.trim() }
        : { kind: "local", model },
    };
    try {
      const transport = await getTransport();
      const result = await transport.completeCommissioning(request);
      setApiKey("");
      if (!result.success) {
        setError(result.error || "We couldn’t finish commissioning. Please try again.");
        return;
      }
      if (result.data?.restartRequired) {
        setRestartPending(true);
        return;
      }
      navigate("/", { replace: true });
    } catch {
      setApiKey("");
      setError("We couldn’t finish commissioning. Please try again.");
    } finally {
      setIsSubmitting(false);
    }
  };

  const checkActivation = async () => {
    setIsCheckingActivation(true);
    setError(null);
    try {
      const transport = await getTransport();
      const result = await transport.getCommissioningStatus();
      if (result.success && result.data?.state === "complete" && !result.data.restartRequired) {
        navigate("/", { replace: true });
        return;
      }
      if (result.data?.recoveryCode === "memory_profile_conflict") {
        setRestartPending(false);
        setError("The Full Zbot memory files or settings changed before activation. Resolve the conflict, then submit commissioning again; z-Bot did not overwrite the existing configuration.");
        return;
      }
      setError("The Full Zbot memory profile is still waiting for a daemon restart.");
    } catch {
      setError("We couldn’t reconnect to z-Bot yet. Restart the daemon, then check activation.");
    } finally {
      setIsCheckingActivation(false);
    }
  };

  if (restartPending) {
    return (
      <main className="commissioning-shell commissioning-shell--activation">
        <section className="commissioning-main commissioning-activation">
          <p className="commissioning-main__eyebrow">Full Zbot memory prepared</p>
          <h1>Restart z-Bot to activate memory</h1>
          <p>Your memory profile will activate when the daemon starts again. The current process will not claim the new recall and background workers are active early.</p>
          <p>Restart zbotd, then check activation here. If the built-in embedding model is not already cached, its first use may complete the one-time local download.</p>
          {error && <p className="commissioning-error" role="alert">{error}</p>}
          <button className="btn btn--primary btn--md" onClick={() => void checkActivation()} disabled={isCheckingActivation}>
            {isCheckingActivation ? <Loader2 className="loading-spinner__icon" /> : <RefreshCw size={16} />} Check activation
          </button>
        </section>
      </main>
    );
  }

  return (
    <main className="commissioning-shell">
      <aside className="commissioning-rail" aria-label="Commissioning progress">
        <div className="commissioning-rail__brand"><Bot aria-hidden="true" /> <span>z-Bot</span></div>
        <p className="commissioning-rail__eyebrow">Agent commissioning</p>
        <ol className="commissioning-progress">
          {STEPS.map((item, index) => (
            <li key={item.id} className={`commissioning-progress__item ${index === currentIndex ? "commissioning-progress__item--active" : ""} ${index < currentIndex ? "commissioning-progress__item--done" : ""}`}>
              <span className="commissioning-progress__number">{index < currentIndex ? <Check size={15} /> : index + 1}</span>
              <span>{item.label}</span>
            </li>
          ))}
        </ol>
        <div className="commissioning-rail__privacy"><LockKeyhole size={17} /><span><strong>Private by default</strong>Your profile stays in local z-Bot data.</span></div>
      </aside>

      <section className="commissioning-main">
        <header className="commissioning-main__header">
          <p className="commissioning-main__eyebrow">Step {currentIndex + 1} of {STEPS.length}</p>
          <h1>{step === "focus" ? "Let’s commission your agent" : STEPS[currentIndex].label}</h1>
          <p>{step === "focus" ? "A few thoughtful choices create an agent that starts in your world." : "You can revisit every choice later in Settings."}</p>
        </header>

        {step === "focus" && (
          <div className="commissioning-panel">
            <h2>What do you want your agent to help you do?</h2>
            <div className="commissioning-focus-grid">
              {FOCUSES.map((item) => {
                const Icon = item.icon;
                const selected = item.id === focus;
                return <button key={item.id} className={`commissioning-focus ${selected ? "commissioning-focus--selected" : ""}`} onClick={() => selectFocus(item.id)} aria-pressed={selected}>
                  <Icon aria-hidden="true" />
                  <strong>{item.title}</strong>
                  <span>{item.description}</span>
                </button>;
              })}
            </div>
            <div className="commissioning-domains">
              <div><h3>Confirm the domains it should understand</h3><p>Your focus suggests a starting set. Keep, remove, or add a domain.</p></div>
              <div className="commissioning-domain-list">
                {DOMAINS.map((domain) => <button key={domain.id} className={`commissioning-domain ${domains.includes(domain.id) ? "commissioning-domain--selected" : ""}`} onClick={() => toggleDomain(domain.id)} aria-pressed={domains.includes(domain.id)}>{domains.includes(domain.id) && <Check size={14} />} {domain.label}</button>)}
              </div>
            </div>
          </div>
        )}

        {step === "intelligence" && (
          <div className="commissioning-panel">
            <h2>Choose how your agent thinks</h2>
            <p className="commissioning-panel__intro">Use a cloud provider with your own key or connect a local Ollama model.</p>
            <div className="commissioning-choice-row">
              <button className={`commissioning-choice ${providerKind === "cloud" ? "commissioning-choice--selected" : ""}`} onClick={() => { setProviderKind("cloud"); setApiKey(""); setModel(CLOUD_PROVIDERS[preset].models[0]); setError(null); }}><Cloud aria-hidden="true" /><strong>Cloud provider</strong><span>Bring an API key from a supported provider.</span></button>
              <button className={`commissioning-choice ${providerKind === "local" ? "commissioning-choice--selected" : ""}`} onClick={() => { setProviderKind("local"); setApiKey(""); setModel(localDiagnosis?.models?.[0] || ""); setError(null); }}><HeartHandshake aria-hidden="true" /><strong>Local model</strong><span>Run privately through Ollama on this device.</span></button>
            </div>
            {providerKind === "cloud" ? (
              <div className="commissioning-provider-form">
                <div className="commissioning-provider-list">
                  {(Object.keys(CLOUD_PROVIDERS) as CloudPreset[]).map((id) => <button key={id} className={`commissioning-provider ${preset === id ? "commissioning-provider--selected" : ""}`} onClick={() => choosePreset(id)} aria-pressed={preset === id}>{CLOUD_PROVIDERS[id].name}</button>)}
                </div>
                <p className="commissioning-provider-form__hint">Need a provider with a custom endpoint or authentication method? Add it in Settings after commissioning.</p>
                <label className="form-group"><span className="form-label">API key</span><input className="form-input" type="password" value={apiKey} onChange={(event) => setApiKey(event.target.value)} autoComplete="off" placeholder="Paste your key" /></label>
                <label className="form-group"><span className="form-label">Model</span><select className="form-select" value={model} onChange={(event) => setModel(event.target.value)}>{selectedModelOptions.map((option) => <option key={option}>{option}</option>)}</select></label>
                {preset === "ollama_cloud" && <div className="commissioning-memory-disclosure" role="note" aria-label="Ollama Cloud model recommendation"><strong>Recommended Ollama Cloud setup</strong><p>All agents start with <code>glm-5.2:cloud</code>. Images and other multimodal work use <code>gemma4:31b-cloud</code>.</p></div>}
              </div>
            ) : (
              <div className="commissioning-local">
                <div><h3>Check your local setup</h3><p>{localDiagnosis ? localMessage(localDiagnosis) : "We’ll look for Ollama and the models already available on your device."}</p></div>
                <button className="btn btn--secondary btn--sm" onClick={() => void diagnoseLocal()} disabled={isDiagnosing}>{isDiagnosing ? <Loader2 className="loading-spinner__icon" /> : <RefreshCw size={15} />} Check local setup</button>
                {localDiagnosis?.models && <label className="form-group"><span className="form-label">Local model</span><select className="form-select" value={model} onChange={(event) => setModel(event.target.value)}>{localDiagnosis.models.map((option) => <option key={option}>{option}</option>)}</select></label>}
              </div>
            )}
          </div>
        )}

        {step === "memory" && (
          <div className="commissioning-panel">
            <h2>Choose how z-Bot remembers</h2>
            <p className="commissioning-panel__intro">Start conservatively, or enable the complete memory and recall profile used by z-Bot.</p>
            <div className="commissioning-choice-row">
              <button type="button" className={`commissioning-choice ${memoryProfile === "zbot_recommended_v1" ? "commissioning-choice--selected" : ""}`} onClick={() => setMemoryProfile("zbot_recommended_v1")} aria-pressed={memoryProfile === "zbot_recommended_v1"}>
                <Sparkles aria-hidden="true" />
                <strong>Full Zbot memory <span className="commissioning-recommended">Recommended</span></strong>
                <span>Enables background memory processing, richer recall, beliefs, hierarchy, and governance.</span>
              </button>
              <button type="button" className={`commissioning-choice ${memoryProfile === "safe_baseline" ? "commissioning-choice--selected" : ""}`} onClick={() => setMemoryProfile("safe_baseline")} aria-pressed={memoryProfile === "safe_baseline"}>
                <ShieldCheck aria-hidden="true" />
                <strong>Safe baseline</strong>
                <span>Keeps current memory settings and does not add recall or governance profile files.</span>
              </button>
            </div>
            <div className="commissioning-memory-disclosure" role="note">
              <strong>Before you enable Full Zbot memory</strong>
              <p>Background memory work uses your selected model provider, consumes usage, and may add cost. {providerKind === "cloud" ? "Memory-derived content may be sent to that cloud provider for processing." : "Memory-derived content stays with your selected local model runtime."} The built-in FastEmbed model may need a one-time local download on first use.</p>
            </div>
          </div>
        )}

        {step === "world" && (
          <div className="commissioning-panel commissioning-panel--narrow">
            <h2>A little about you</h2>
            <p className="commissioning-panel__intro">This helps z-Bot begin in your world. Your personal details stay only in local z-Bot data on this device—there is no online z-Bot profile. You can change or delete them later.</p>
            <label className="form-group"><span className="form-label">Your name</span><input className="form-input" value={userName} onChange={(event) => setUserName(event.target.value)} maxLength={80} autoComplete="name" placeholder="What should z-Bot call you?" /></label>
            <div className="form-group"><span className="form-label">A few interests <em>Choose at least one</em></span><div className="commissioning-domain-list">{INTERESTS.map((interest) => <button key={interest} type="button" className={`commissioning-domain ${interests.includes(interest) ? "commissioning-domain--selected" : ""}`} onClick={() => toggleInterest(interest)} aria-pressed={interests.includes(interest)}>{interests.includes(interest) && <Check size={14} />} {interest}</button>)}</div></div>
            <label className="form-group"><span className="form-label">Hobbies & pastimes <em>Optional</em></span><input className="form-input" value={hobbies} onChange={(event) => setHobbies(event.target.value)} maxLength={960} placeholder="For example: hiking, cooking, photography" /></label>
            <label className="form-group"><span className="form-label">Date of birth <em>Optional</em></span><input className="form-input" type="date" value={dateOfBirth} onChange={(event) => setDateOfBirth(event.target.value)} autoComplete="bday" /></label>
            <label className="form-group"><span className="form-label">What should we call your agent?</span><input className="form-input" value={displayName} onChange={(event) => setDisplayName(event.target.value)} maxLength={80} /></label>
            <label className="form-group"><span className="form-label">Anything it should know about how you work? <em>Optional</em></span><textarea className="form-textarea" value={profile} onChange={(event) => setProfile(event.target.value)} maxLength={2000} placeholder="For example: I prefer concise technical answers and I’m learning Rust." rows={5} /></label>
          </div>
        )}

        {error && <p className="commissioning-error" role="alert">{error}</p>}
        <footer className="commissioning-actions">
          {currentIndex > 0 ? <button className="btn btn--ghost btn--md" onClick={back}>Back</button> : <span />}
          {step === "world" ? <button className="btn btn--primary btn--md" onClick={() => void complete()} disabled={!canContinue || isSubmitting}>{isSubmitting ? <Loader2 className="loading-spinner__icon" /> : <>Commission my agent <ArrowRight size={17} /></>}</button> : <button className="btn btn--primary btn--md" onClick={next} disabled={!canContinue}>Continue <ArrowRight size={17} /></button>}
        </footer>
      </section>

      <aside className="commissioning-prepare" aria-label="Commissioning summary">
        <h2>What we’ll prepare</h2>
        <ul>
          <li><Bot aria-hidden="true" /> Your agent profile</li>
          <li><Sparkles aria-hidden="true" /> A private memory plan</li>
          {memoryProfile === "zbot_recommended_v1" && <li><Check aria-hidden="true" /> A starter taxonomy</li>}
          {memoryProfile === "zbot_recommended_v1" && <li><Check aria-hidden="true" /> A working ontology</li>}
          <li><ShieldCheck aria-hidden="true" /> Safe defaults</li>
        </ul>
        <div className="commissioning-safety-notice" role="note"><ShieldCheck aria-hidden="true" /><div><strong>An autonomous agent, with guardrails</strong><p>z-Bot can plan and use the tools you enable. Safety guardrails are built in, but execution is not sandboxed yet: a command or tool can affect this device and connected services. Review requests and enable only integrations you trust.</p></div></div>
        {focusDetails && <p className="commissioning-prepare__selection">Starting with <strong>{focusDetails.title}</strong> and {domains.length} confirmed {domains.length === 1 ? "domain" : "domains"}.</p>}
      </aside>
    </main>
  );
}

function localMessage(diagnosis: LocalDiagnosis): string {
  switch (diagnosis.state) {
    case "unavailable": return "Ollama is not installed yet. Install it, then check again.";
    case "unreachable": return "Ollama is installed but not responding. Start it, then check again.";
    case "no_model": return "Ollama is ready, but no model is available. Download one in Ollama, then check again.";
    case "ready": return "Your local runtime is ready. Choose a model to continue.";
  }
}

function splitList(value: string): string[] {
  return value.split(",").map((item) => item.trim()).filter(Boolean);
}
