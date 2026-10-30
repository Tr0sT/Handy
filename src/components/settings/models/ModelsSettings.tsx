import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import {
  AudioLines,
  ChevronDown,
  Download,
  Globe,
  KeyRound,
  Languages,
  Loader2,
  LogOut,
  RefreshCw,
  Search,
} from "lucide-react";
import type { ModelCardStatus } from "@/components/onboarding";
import { ModelCard } from "@/components/onboarding";
import { useModelStore } from "@/stores/modelStore";
import {
  getLanguageLabel,
  MODEL_CAPABILITY_LANGUAGES,
  supportsLanguageCode,
} from "@/lib/constants/languages.ts";
import { commands, type CodexAuthState, type ModelInfo } from "@/bindings";

const CODEX_LOCAL_MODEL_ID = "codex-local";

/** Past this many models on disk, the rest fold under "Show N more". */
const YOURS_LIMIT = 5;

/** Look for model files added outside Handy: quiet, beside its section's name. */
const Rescan: React.FC = () => {
  const { t } = useTranslation();
  const rescanning = useModelStore((s) => s.isRescanning);
  return (
    <Tooltip text={t("settings.models.rescan.tooltip")}>
      <button
        type="button"
        onClick={() => useModelStore.getState().rescanLocalModels()}
        disabled={rescanning}
        aria-label={t("settings.models.rescan.tooltip")}
        className="ms-[6px] grid size-[22px] cursor-pointer place-items-center rounded-full text-ink-3 hover:bg-wash hover:text-text disabled:cursor-default focus:outline-none focus-visible:ring-1 focus-visible:ring-logo-primary"
      >
        <RefreshCw
          className={`size-[12px] ${rescanning ? "animate-spin" : ""}`}
          aria-hidden="true"
        />
      </button>
    </Tooltip>
  );
};

interface ModelsSettingsProps {
  /** Where the languages are changed (General). */
  onChangeLanguages?: () => void;
}

const CodexLocalAuthPanel: React.FC = () => {
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

export const ModelsSettings: React.FC = () => {
  const { t } = useTranslation();
  const loading = useModelStore((s) => s.loading);
  const { facts, lists } = useModelEntries();
  const { filters, setFilters, searching, matches } = useModelFilters();
  const [more, setMore] = useState(false);

  const renderModelCard = (model: ModelInfo, showRecommended: boolean) => (
    <ModelCard
      key={model.id}
      model={model}
      status={getModelStatus(model.id)}
      onSelect={handleModelSelect}
      onDownload={handleModelDownload}
      onDelete={
        model.id === CODEX_LOCAL_MODEL_ID ? undefined : handleModelDelete
      }
      onCancel={handleModelCancel}
      downloadProgress={getDownloadProgress(model.id)}
      downloadSpeed={getDownloadSpeed(model.id)}
      showRecommended={showRecommended}
    >
      {model.id === CODEX_LOCAL_MODEL_ID && <CodexLocalAuthPanel />}
    </ModelCard>
  );

  if (loading) {
    return (
      <div className="max-w-3xl w-full mx-auto">
        <div className="flex items-center justify-center py-16">
          <div className="w-8 h-8 border-2 border-logo-primary border-t-transparent rounded-full animate-spin" />
        </div>
      </div>
    );
  }

  const keep = (xs: typeof lists.yours) => xs.filter(matches);
  const moreButton = !searching && lists.yours.length > YOURS_LIMIT && (
    <button
      type="button"
      onClick={() => setMore((m) => !m)}
      className="ms-[40px] mt-[2px] flex h-[28px] cursor-pointer items-center gap-[8px] rounded-[7px] px-[2px] text-small font-medium text-ink-2 hover:text-text focus:outline-none focus-visible:ring-1 focus-visible:ring-logo-primary"
    >
      {more
        ? t("settings.models.showFewer")
        : t("settings.models.showMore", {
            count: lists.yours.length - YOURS_LIMIT,
          })}
      <span
        aria-hidden="true"
        className={`inline-block size-[6px] shrink-0 border-e-[1.5px] border-b-[1.5px] border-current ${more ? "mt-[3px] -rotate-[135deg]" : "-mt-[3px] rotate-45"}`}
      />
    </button>
  );

  const sections: TableSection[] = searching
    ? [
        {
          key: "yours",
          label: t("settings.models.sections.yours"),
          rows: keep(lists.yours),
        },
        {
          key: "fits",
          label: t("settings.models.sections.fits"),
          rows: keep(lists.fits),
        },
        {
          key: "partial",
          label: t("modelTable.missingLanguage"),
          rows: keep(lists.partial),
        },
        {
          key: "untested",
          label: t("settings.models.sections.untested"),
          rows: filters.query.trim() ? keep(lists.untested) : [],
          note: t("settings.models.untestedNote"),
        },
      ]
    : [
        {
          key: "yours",
          label: (
            <>
              {t("settings.models.sections.yours")}
              <Rescan />
            </>
          ),
          rows: more ? lists.yours : lists.yours.slice(0, YOURS_LIMIT),
          after: lists.yours.length ? (
            moreButton
          ) : (
            <Note>{t("settings.models.emptyYours")}</Note>
          ),
        },
        {
          key: "fits",
          label: t("settings.models.sections.fits"),
          rows: lists.fits,
        },
        {
          key: "partial",
          label: t("modelTable.missingLanguage"),
          rows: lists.partial,
        },
      ];

  return (
    <div className="max-w-3xl w-full mx-auto space-y-4">
      <div className="mb-4">
        <h1 className="text-xl font-semibold mb-2">
          {t("settings.models.title")}
        </h1>
        <p className="text-sm text-text/60">
          {t("settings.models.description")}
        </p>
      </div>

      {/* Search bar — filter the catalog by name or description */}
      <div className="relative">
        <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-text/40 pointer-events-none" />
        <input
          type="text"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          placeholder={t("settings.models.searchPlaceholder")}
          className="w-full pl-9 pr-3 py-2 text-sm bg-mid-gray/10 border border-mid-gray/40 rounded-lg focus:outline-none focus:ring-1 focus:ring-logo-primary placeholder:text-text/40"
        />
      </div>

      <div className="space-y-6">
        {/* Downloaded Models Section — header always visible so filter stays accessible */}
        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-medium text-text/60">
              {t("settings.models.yourModels")}
            </h2>
            <div className="flex items-center gap-2">
              {/* Rescan local sources for models added outside Handy */}
              <button
                type="button"
                onClick={() => rescanLocalModels()}
                disabled={isRescanning}
                title={t("settings.models.rescan.tooltip")}
                aria-label={t("settings.models.rescan.tooltip")}
                className="flex items-center justify-center w-8 h-8 text-sm font-medium rounded-lg bg-mid-gray/10 text-text/60 hover:bg-mid-gray/20 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                <RefreshCw
                  className={`w-3.5 h-3.5 ${isRescanning ? "animate-spin" : ""}`}
                />
              </button>

              {/* Vertical divider separating action from filters */}
              <div className="h-4 w-px bg-mid-gray/30 mx-0.5" />
              <button
                type="button"
                onClick={() => setFilterStreaming((enabled) => !enabled)}
                title={t("settings.models.filters.streaming")}
                aria-label={t("settings.models.filters.streaming")}
                aria-pressed={filterStreaming}
                className={`flex items-center justify-center w-8 h-8 text-sm font-medium rounded-lg transition-colors ${
                  filterStreaming
                    ? "bg-logo-primary/20 text-logo-primary hover:bg-logo-primary/30"
                    : "bg-mid-gray/10 text-text/60 hover:bg-mid-gray/20"
                }`}
              >
                <AudioLines className="w-3.5 h-3.5" />
              </button>
              <button
                type="button"
                onClick={() => setFilterTranslation((enabled) => !enabled)}
                title={t("settings.models.filters.translation")}
                aria-label={t("settings.models.filters.translation")}
                aria-pressed={filterTranslation}
                className={`flex items-center justify-center w-8 h-8 text-sm font-medium rounded-lg transition-colors ${
                  filterTranslation
                    ? "bg-logo-primary/20 text-logo-primary hover:bg-logo-primary/30"
                    : "bg-mid-gray/10 text-text/60 hover:bg-mid-gray/20"
                }`}
              >
                <Languages className="w-3.5 h-3.5" />
              </button>
              {/* Language filter dropdown */}
              <div className="relative" ref={languageDropdownRef}>
                <button
                  type="button"
                  onClick={() => setLanguageDropdownOpen(!languageDropdownOpen)}
                  className={`flex items-center gap-1.5 h-8 px-3 text-sm font-medium rounded-lg transition-colors ${
                    languageFilter !== "all"
                      ? "bg-logo-primary/20 text-logo-primary"
                      : "bg-mid-gray/10 text-text/60 hover:bg-mid-gray/20"
                  }`}
                >
                  <Globe className="w-3.5 h-3.5" />
                  <span className="max-w-[120px] truncate">
                    {selectedLanguageLabel}
                  </span>
                  <ChevronDown
                    className={`w-3.5 h-3.5 transition-transform ${
                      languageDropdownOpen ? "rotate-180" : ""
                    }`}
                  />
                </button>

                {languageDropdownOpen && (
                  <div className="absolute top-full right-0 mt-1 w-56 bg-background border border-mid-gray/80 rounded-lg shadow-lg z-50 overflow-hidden">
                    <div className="p-2 border-b border-mid-gray/40">
                      <input
                        ref={languageSearchInputRef}
                        type="text"
                        value={languageSearch}
                        onChange={(e) => setLanguageSearch(e.target.value)}
                        onKeyDown={(e) => {
                          if (
                            e.key === "Enter" &&
                            filteredLanguages.length > 0
                          ) {
                            setLanguageFilter(filteredLanguages[0].value);
                            setLanguageDropdownOpen(false);
                            setLanguageSearch("");
                          } else if (e.key === "Escape") {
                            setLanguageDropdownOpen(false);
                            setLanguageSearch("");
                          }
                        }}
                        placeholder={t(
                          "settings.general.language.searchPlaceholder",
                        )}
                        className="w-full px-2 py-1 text-sm bg-mid-gray/10 border border-mid-gray/40 rounded-md focus:outline-none focus:ring-1 focus:ring-logo-primary"
                      />
                    </div>
                    <div className="max-h-48 overflow-y-auto">
                      <button
                        type="button"
                        onClick={() => {
                          setLanguageFilter("all");
                          setLanguageDropdownOpen(false);
                          setLanguageSearch("");
                        }}
                        className={`w-full px-3 py-1.5 text-sm text-left transition-colors ${
                          languageFilter === "all"
                            ? "bg-logo-primary/20 text-logo-primary font-semibold"
                            : "hover:bg-mid-gray/10"
                        }`}
                      >
                        {t("settings.models.filters.allLanguages")}
                      </button>
                      {filteredLanguages.map((lang) => (
                        <button
                          key={lang.value}
                          type="button"
                          onClick={() => {
                            setLanguageFilter(lang.value);
                            setLanguageDropdownOpen(false);
                            setLanguageSearch("");
                          }}
                          className={`w-full px-3 py-1.5 text-sm text-left transition-colors ${
                            languageFilter === lang.value
                              ? "bg-logo-primary/20 text-logo-primary font-semibold"
                              : "hover:bg-mid-gray/10"
                          }`}
                        >
                          {lang.label}
                        </button>
                      ))}
                      {filteredLanguages.length === 0 && (
                        <div className="px-3 py-2 text-sm text-text/50 text-center">
                          {t("settings.general.language.noResults")}
                        </div>
                      )}
                    </div>
                  </div>
                )}
              </div>
            </div>
          </div>
          {downloadedModels.map((model: ModelInfo) =>
            renderModelCard(model, false),
          )}
        </div>

        {/* Available Models Section */}
        {availableModels.length > 0 && (
          <div className="space-y-3">
            <h2 className="text-sm font-medium text-text/60">
              {t("settings.models.availableModels")}
            </h2>
            {availableModels.map((model: ModelInfo) =>
              renderModelCard(model, true),
            )}
          </div>
        )}
        {filteredModels.length === 0 && (
          <div className="text-center py-8 text-text/50">
            {t("settings.models.noModelsMatch")}
          </div>
        )}
      </div>
      <div className="mx-[12px] mb-[16px] flex-none empty:hidden">
        <RetiredModelsNotice />
      </div>
      <ModelTable
        grading={facts}
        filters={filters}
        setFilters={setFilters}
        // "Yours" always shows at rest; a search hides empty sections.
        sections={sections.filter(
          (s) => s.rows.length || (s.key === "yours" && !searching),
        )}
        renderRow={(e) => <ModelRow key={e.id} entry={e} />}
        actionsHead={<Legend tier={facts.tier} />}
        footer={
          !searching &&
          lists.untested.length > 0 && (
            <Note>
              {t("settings.models.untestedHidden", {
                count: lists.untested.length,
              })}
            </Note>
          )
        }
        thinEdge
      />
    </div>
  );
};
