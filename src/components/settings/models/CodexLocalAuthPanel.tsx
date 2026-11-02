import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Download, KeyRound, Loader2, LogOut } from "lucide-react";
import { commands, type CodexAuthState } from "@/bindings";

export const CodexLocalAuthPanel: React.FC = () => {
  const { t } = useTranslation();
  const [auth, setAuth] = useState<CodexAuthState | null>(null);
  const [tokenInput, setTokenInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [showManual, setShowManual] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadAuthState = async () => {
    const result = await commands.getCodexAuthState();
    if (result.status === "ok") {
      setAuth(result.data);
    }
  };

  useEffect(() => {
    loadAuthState();
  }, []);

  const handleImport = async () => {
    setLoading(true);
    setError(null);
    const result = await commands.importCodexCredentials();
    if (result.status === "ok") {
      await loadAuthState();
    } else {
      setError(result.error);
    }
    setLoading(false);
  };

  const handleManualLogin = async () => {
    const token = tokenInput.trim();
    if (!token) return;
    setLoading(true);
    setError(null);
    const result = await commands.setCodexAccessToken(token);
    if (result.status === "ok") {
      await loadAuthState();
      setTokenInput("");
      setShowManual(false);
    } else {
      setError(result.error);
    }
    setLoading(false);
  };

  const handleLogout = async () => {
    setLoading(true);
    setError(null);
    const result = await commands.codexLogout();
    if (result.status === "ok") {
      await loadAuthState();
    } else {
      setError(result.error);
    }
    setLoading(false);
  };

  return (
    <div className="flex flex-col gap-2 text-sm">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 min-w-0">
          <div
            className={`w-2 h-2 rounded-full shrink-0 ${
              auth?.is_logged_in ? "bg-green-500" : "bg-red-400"
            }`}
          />
          <span className="text-text/70 truncate">
            {auth?.is_logged_in
              ? t("settings.models.codexLocal.connected", "Connected to Codex")
              : t(
                  "settings.models.codexLocal.notConnected",
                  "Not connected to Codex",
                )}
          </span>
        </div>
        {auth?.is_logged_in ? (
          <button
            type="button"
            onClick={handleLogout}
            disabled={loading}
            className="flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md bg-red-500/10 hover:bg-red-500/20 text-red-400 transition-colors disabled:opacity-50"
          >
            <LogOut className="w-3.5 h-3.5" />
            {t("settings.models.codexLocal.disconnect", "Disconnect")}
          </button>
        ) : (
          <button
            type="button"
            onClick={handleImport}
            disabled={loading}
            className="flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md bg-logo-primary/20 hover:bg-logo-primary/30 transition-colors disabled:opacity-50"
          >
            {loading ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <Download className="w-3.5 h-3.5" />
            )}
            {t("settings.models.codexLocal.import", "Import credentials")}
          </button>
        )}
      </div>

      {!auth?.is_logged_in && (
        <button
          type="button"
          onClick={() => setShowManual((value) => !value)}
          className="flex items-center gap-1.5 text-xs text-text/50 hover:text-text/80 transition-colors self-start"
        >
          <KeyRound className="w-3.5 h-3.5" />
          {t("settings.models.codexLocal.manualToken", "Enter token manually")}
        </button>
      )}

      {showManual && !auth?.is_logged_in && (
        <div className="flex gap-2">
          <input
            type="password"
            value={tokenInput}
            onChange={(event) => setTokenInput(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") handleManualLogin();
            }}
            placeholder={t(
              "settings.models.codexLocal.tokenPlaceholder",
              "Paste token here...",
            )}
            className="flex-1 min-w-0 px-3 py-1.5 text-sm rounded-md border border-mid-gray/30 bg-transparent focus:outline-none focus:border-logo-primary"
          />
          <button
            type="button"
            onClick={handleManualLogin}
            disabled={loading || !tokenInput.trim()}
            className="px-3 py-1.5 text-sm rounded-md bg-logo-primary hover:bg-logo-primary/80 disabled:opacity-50 transition-colors"
          >
            {t("common.save", "Save")}
          </button>
        </div>
      )}

      {error && <p className="text-xs text-red-400">{error}</p>}
    </div>
  );
};
