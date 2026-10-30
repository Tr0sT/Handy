/**
 * Spoken languages from the model catalog. Codes are `languageIdentity`s,
 * one spelling per language (no/nb → nb), compared and saved as is.
 */
import { catalog, languageIdentity } from "@handy-computer/models";

/** `native` is the endonym, never translated. UI names: `languageName`. */
export interface Language {
  code: string;
  native: string;
}

// Intl names "nb" Norwegian Bokmål; people say Norwegian.
const NAME_CODES: Record<string, string> = { nb: "no" };

/**
 * Claude.ai voice pipeline supported language codes (20 languages).
 * Extracted from Claude Code's voice_stream module — the backend only
 * accepts these codes via the `language` query parameter.
 */
export const CLAUDE_VOICE_LANGUAGES = new Set([
  "en",
  "es",
  "fr",
  "ja",
  "de",
  "pt",
  "it",
  "ko",
  "hi",
  "id",
  "ru",
  "pl",
  "tr",
  "nl",
  "uk",
  "el",
  "cs",
  "da",
  "sv",
  "no",
]);

/**
 * Full Whisper language list — used by local models and Codex (ChatGPT Whisper).
 */
export const LANGUAGES: Language[] = [
  { value: "auto", label: "Auto Detect" },
  { value: "en", label: "English" },
  { value: "zh", label: "Chinese (Mandarin)" },
  { value: "yue", label: "Cantonese" },
  { value: "de", label: "German" },
  { value: "es", label: "Spanish" },
  { value: "ru", label: "Russian" },
  { value: "ko", label: "Korean" },
  { value: "fr", label: "French" },
  { value: "ja", label: "Japanese" },
  { value: "pt", label: "Portuguese" },
  { value: "tr", label: "Turkish" },
  { value: "pl", label: "Polish" },
  { value: "ca", label: "Catalan" },
  { value: "nl", label: "Dutch" },
  { value: "ar", label: "Arabic" },
  { value: "sv", label: "Swedish" },
  { value: "it", label: "Italian" },
  { value: "id", label: "Indonesian" },
  { value: "hi", label: "Hindi" },
  { value: "fi", label: "Finnish" },
  { value: "vi", label: "Vietnamese" },
  { value: "he", label: "Hebrew" },
  { value: "uk", label: "Ukrainian" },
  { value: "el", label: "Greek" },
  { value: "ms", label: "Malay" },
  { value: "cs", label: "Czech" },
  { value: "ro", label: "Romanian" },
  { value: "da", label: "Danish" },
  { value: "hu", label: "Hungarian" },
  { value: "ta", label: "Tamil" },
  { value: "no", label: "Norwegian" },
  { value: "th", label: "Thai" },
  { value: "ur", label: "Urdu" },
  { value: "hr", label: "Croatian" },
  { value: "bg", label: "Bulgarian" },
  { value: "lt", label: "Lithuanian" },
  { value: "la", label: "Latin" },
  { value: "mi", label: "Maori" },
  { value: "ml", label: "Malayalam" },
  { value: "cy", label: "Welsh" },
  { value: "sk", label: "Slovak" },
  { value: "te", label: "Telugu" },
  { value: "fa", label: "Persian" },
  { value: "lv", label: "Latvian" },
  { value: "bn", label: "Bengali" },
  { value: "sr", label: "Serbian" },
  { value: "az", label: "Azerbaijani" },
  { value: "sl", label: "Slovenian" },
  { value: "kn", label: "Kannada" },
  { value: "et", label: "Estonian" },
  { value: "mk", label: "Macedonian" },
  { value: "br", label: "Breton" },
  { value: "eu", label: "Basque" },
  { value: "is", label: "Icelandic" },
  { value: "ga", label: "Irish" },
  { value: "hy", label: "Armenian" },
  { value: "ne", label: "Nepali" },
  { value: "mn", label: "Mongolian" },
  { value: "bs", label: "Bosnian" },
  { value: "kk", label: "Kazakh" },
  { value: "sq", label: "Albanian" },
  { value: "sw", label: "Swahili" },
  { value: "gl", label: "Galician" },
  { value: "mr", label: "Marathi" },
  { value: "pa", label: "Punjabi" },
  { value: "si", label: "Sinhala" },
  { value: "km", label: "Khmer" },
  { value: "sn", label: "Shona" },
  { value: "yo", label: "Yoruba" },
  { value: "so", label: "Somali" },
  { value: "af", label: "Afrikaans" },
  { value: "oc", label: "Occitan" },
  { value: "ka", label: "Georgian" },
  { value: "be", label: "Belarusian" },
  { value: "tg", label: "Tajik" },
  { value: "sd", label: "Sindhi" },
  { value: "gu", label: "Gujarati" },
  { value: "am", label: "Amharic" },
  { value: "yi", label: "Yiddish" },
  { value: "lo", label: "Lao" },
  { value: "uz", label: "Uzbek" },
  { value: "fo", label: "Faroese" },
  { value: "ht", label: "Haitian Creole" },
  { value: "ps", label: "Pashto" },
  { value: "tk", label: "Turkmen" },
  { value: "nn", label: "Nynorsk" },
  { value: "mt", label: "Maltese" },
  { value: "sa", label: "Sanskrit" },
  { value: "lb", label: "Luxembourgish" },
  { value: "my", label: "Myanmar" },
  { value: "bo", label: "Tibetan" },
  { value: "tl", label: "Filipino (Tagalog)" },
  { value: "mg", label: "Malagasy" },
  { value: "as", label: "Assamese" },
  { value: "tt", label: "Tatar" },
  { value: "haw", label: "Hawaiian" },
  { value: "ln", label: "Lingala" },
  { value: "ha", label: "Hausa" },
  { value: "ba", label: "Bashkir" },
  { value: "jw", label: "Javanese" },
  { value: "su", label: "Sundanese" },
];

const LANGUAGE_LABELS = new Map(
  LANGUAGES.map((language) => [language.value, language.label] as const),
);

export const MODEL_CAPABILITY_LANGUAGES: Language[] = LANGUAGES.filter(
  (language) => language.value !== "auto",
);

// Collapse a language tag to the canonical recognition intent Handy exposes in
// the UI. BCP-47 region/script subtags are dropped ("en-US" → "en",
// "zh-Hant" → "zh"), and model-specific base-code equivalents are mapped to a
// stable intent. Norwegian Bokmål (`nb`) maps to Norwegian (`no`), while
// Nynorsk (`nn`) remains distinct; Filipino (`fil`) maps to Tagalog (`tl`). The
// backend performs the same equivalence match but returns the model's real code
// so the engine always receives exactly what it advertises.
export const recognitionLanguage = (languageCode: string): string => {
  const separatorIndex = languageCode.indexOf("-");
  const baseCode =
    separatorIndex === -1
      ? languageCode
      : languageCode.slice(0, separatorIndex);

  return LANGUAGE_ALIASES.get(baseCode) ?? baseCode;
};

/** Languages' names as a list in the UI language ("English and Chinese"). */
export const languageList = (codes: string[], uiLanguage: string): string =>
  new Intl.ListFormat(uiLanguage, { type: "conjunction" }).format(
    codes.map((code) => languageName(code, uiLanguage)),
  );

// Scripts that capitalize a name's first letter. (Georgian has capitals but
// doesn't start words with them.)
const CASED =
  /^[\p{Script=Latin}\p{Script=Cyrillic}\p{Script=Greek}\p{Script=Armenian}]/u;

/** A language's name in itself, capitalized as a heading ("Français"). */
const endonym = (code: string): string => {
  const name = languageName(code, code);
  return CASED.test(name)
    ? name.charAt(0).toLocaleUpperCase(code) + name.slice(1)
    : name;
};

// How many of the catalog's speech models hear each language.
const SUPPORT = new Map<string, number>();
for (const model of catalog.models.filter((m) => m.role === "asr"))
  for (const code of new Set(model.languages.map(languageIdentity)))
    SUPPORT.set(code, (SUPPORT.get(code) ?? 0) + 1);

const byEnglishName = (a: string, b: string) =>
  languageName(a, "en").localeCompare(languageName(b, "en"));

/** Catalog languages, most supported first, then by English name. */
export const LANGUAGES: Language[] = [...SUPPORT]
  .sort(
    ([a, aModels], [b, bModels]) => bModels - aModels || byEnglishName(a, b),
  )
  .map(([code]) => ({ code, native: endonym(code) }));

const BY_CODE = new Map(LANGUAGES.map((l) => [l.code, l]));
const RANK = new Map(LANGUAGES.map((l, i) => [l.code, i]));

/** The language for any code a model or setting uses ("nb-NO", "jw"). */
export const getLanguage = (code: string): Language => {
  const identity = languageIdentity(code);
  return BY_CODE.get(identity) ?? { code: identity, native: endonym(identity) };
};

/** A model's languages, deduped, in catalog order; unknown codes go last. */
export const languagesOf = (codes: readonly string[]): Language[] =>
  [...new Set(codes.map(languageIdentity))]
    .sort(
      (a, b) =>
        (RANK.get(a) ?? Infinity) - (RANK.get(b) ?? Infinity) ||
        byEnglishName(a, b),
    )
    .map(getLanguage);

/** Whether a model's codes include this language, however it's spelled. */
export const supportsLanguage = (
  codes: readonly string[],
  language: string,
): boolean => {
  const identity = languageIdentity(language);
  return codes.some((code) => languageIdentity(code) === identity);
};

// What people might type that isn't the endonym or the name.
const ALIASES: Record<string, string[]> = {
  zh: [
    "chinese",
    "mandarin",
    "putonghua",
    "guoyu",
    "普通话",
    "國語",
    "汉语",
    "華語",
    "中文",
  ],
  yue: ["cantonese", "广东话", "廣東話", "粤语", "粵語"],
  fa: ["farsi"],
  fil: ["tagalog", "pilipino"],
  nb: ["norwegian", "bokmål", "bokmal"],
  ms: ["malaysian", "melayu"],
  id: ["indonesian", "bahasa"],
  he: ["ivrit"],
  hi: ["hindustani"],
  ja: ["nihongo"],
  ko: ["hangul", "hangugeo"],
};

/** Languages matching a query; a prefix match on any name beats a substring. */
export const searchLanguages = (
  query: string,
  uiLanguage: string,
  languages: readonly Language[] = LANGUAGES,
): Language[] => {
  const q = query.trim().toLowerCase();
  if (!q) return languages.slice();
  const hits: [number, Language][] = [];
  for (const l of languages) {
    const names = [
      l.native,
      languageName(l.code, uiLanguage),
      languageName(l.code, "en"),
      l.code,
      ...(ALIASES[l.code] ?? []),
    ].map((n) => n.toLowerCase());
    const prefix = names.some((n) => n.startsWith(q));
    if (prefix || names.some((n) => n.includes(q)))
      hits.push([prefix ? 0 : 1, l]);
  }
  return hits.sort((a, b) => a[0] - b[0]).map(([, l]) => l);
};
