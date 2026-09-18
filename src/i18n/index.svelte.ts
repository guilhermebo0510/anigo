import { pt_BR } from "./pt_BR";
import { en_US } from "./en_US";
import { ja_JP } from "./ja_JP";

export type LanguageCode = "pt-BR" | "en" | "ja";

const dictionaries: Record<LanguageCode, Record<string, string>> = {
  "pt-BR": pt_BR,
  "en": en_US,
  "ja": ja_JP,
};

class I18nManager {
  current = $state<LanguageCode>(
    (typeof localStorage !== "undefined" && (localStorage.getItem("anigo_language") as LanguageCode)) || "pt-BR"
  );

  getLanguage(): LanguageCode {
    return this.current;
  }

  setLanguage(lang: LanguageCode) {
    this.current = lang;
    if (typeof localStorage !== "undefined") {
      localStorage.setItem("anigo_language", lang);
    }
  }

  t(key: string, defaultText?: string): string {
    const dict = dictionaries[this.current] || pt_BR;
    if (dict && dict[key]) {
      return dict[key];
    }
    if (pt_BR[key]) {
      return pt_BR[key];
    }
    return defaultText || key;
  }
}

export const i18nState = new I18nManager();

export function getLanguage(): LanguageCode {
  return i18nState.getLanguage();
}

export function setLanguage(lang: LanguageCode) {
  i18nState.setLanguage(lang);
}

export function t(key: string, defaultText?: string): string {
  return i18nState.t(key, defaultText);
}
