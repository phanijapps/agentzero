import { useCallback, useEffect, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { AlertTriangle, Loader2, RefreshCw } from "lucide-react";
import { getTransport } from "@/services/transport";

interface CommissioningGuardProps {
  children: React.ReactNode;
}

export function CommissioningGuard({ children }: CommissioningGuardProps) {
  const [state, setState] = useState<"checking" | "ready" | "error">("checking");
  const location = useLocation();
  const navigate = useNavigate();

  const check = useCallback(async () => {
    setState("checking");
    try {
      const transport = await getTransport();
      const result = await transport.getCommissioningStatus();
      if (!result.success || !result.data) {
        setState("error");
        return;
      }
      if (result.data.state !== "complete") {
        navigate("/commission", { replace: true });
        return;
      }
      setState("ready");
    } catch {
      setState("error");
    }
  }, [navigate]);

  useEffect(() => {
    if (location.pathname === "/commission" || location.pathname === "/setup") {
      setState("ready");
      return;
    }
    void check();
  }, [check, location.pathname]);

  if (state === "checking") {
    return (
      <div className="commissioning-guard">
        <Loader2 className="loading-spinner__icon" aria-label="Checking readiness" />
      </div>
    );
  }

  if (state === "error") {
    return (
      <div className="commissioning-guard">
        <div className="commissioning-guard__error" role="alert">
          <AlertTriangle aria-hidden="true" />
          <div>
            <h1>We can’t confirm z-Bot is ready yet</h1>
            <p>Check that the local gateway is running, then try again.</p>
          </div>
          <button className="btn btn--primary btn--sm" onClick={() => void check()}>
            <RefreshCw size={15} aria-hidden="true" /> Retry
          </button>
        </div>
      </div>
    );
  }

  return <>{children}</>;
}
