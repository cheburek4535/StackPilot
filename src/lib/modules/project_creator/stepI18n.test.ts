// Тесты локализации названий шагов Project Creator (stepI18n + словари):
// согласованность RU/EN, точные переводы статичных label и шаблонные правила.
import { describe, expect, it, beforeAll, afterAll } from "vitest";
import en from "../../core/locales/en";
import ru from "../../core/locales/ru";
import { i18n } from "../../core/i18n.svelte";
import type { Locale } from "../../core/i18n.svelte";
import { tStepLabel } from "./stepI18n";

const stepKeys = (): string[] =>
  Object.keys(en).filter((k) => k.startsWith("step."));

const enDict = en as Record<string, string>;
const ruDict = ru as Record<string, string>;

describe("step i18n dictionaries", () => {
  it("en and ru have identical step.* key sets", () => {
    const enKeys = stepKeys().sort();
    const ruKeys = Object.keys(ru)
      .filter((k) => k.startsWith("step."))
      .sort();
    expect(ruKeys).toEqual(enKeys);
  });

  it("en step.* values are identity (match the trailing English text)", () => {
    for (const key of stepKeys()) {
      if (key.startsWith("step.label.")) {
        expect(enDict[key]).toBe(key.slice("step.label.".length));
      }
    }
  });

  it("template keys use the same placeholders in both locales", () => {
    for (const key of stepKeys()) {
      if (!key.startsWith("step.tmpl.")) continue;
      const ph = (s: string) => Array.from(s.matchAll(/\{(\w+)\}/g), (m) => m[1]).sort();
      expect(ph(ruDict[key])).toEqual(ph(enDict[key]));
    }
  });
});

describe("tStepLabel", () => {
  let saved: Locale;
  beforeAll(() => {
    saved = i18n.locale;
  });
  afterAll(() => {
    i18n.setLocale(saved);
  });

  it("translates static labels into Russian by default locale", () => {
    i18n.setLocale("ru");
    expect(tStepLabel("Create project root")).toBe("Создать корень проекта");
    expect(tStepLabel("Check Python interpreter")).toBe("Проверить интерпретатор Python");
    expect(tStepLabel("Initialize Git repository")).toBe("Инициализировать Git-репозиторий");
    expect(tStepLabel("Create initial commit")).toBe("Создать первый коммит");
  });

  it("keeps English labels unchanged in the en locale", () => {
    i18n.setLocale("en");
    expect(tStepLabel("Create project root")).toBe("Create project root");
    expect(tStepLabel("Create MAUI app")).toBe("Create MAUI app");
  });

  it("applies template rules for dynamic labels", () => {
    i18n.setLocale("ru");
    expect(tStepLabel("Create src/index.ts")).toBe("Создать src/index.ts");
    expect(tStepLabel("Create main.go")).toBe("Создать main.go");
    expect(tStepLabel("Install npm dependencies (frontend)")).toBe(
      "Установить npm-зависимости (frontend)",
    );
    expect(tStepLabel("Check maven/gradle toolchain")).toBe("Проверить наличие maven/gradle");
    expect(tStepLabel("Validate vue package.json")).toBe("Проверить package.json для vue");
    expect(tStepLabel("Create config dir for npm")).toBe("Создать папку конфигурации для npm");
    expect(tStepLabel("Create django app")).toBe("Создать приложение django");
  });

  it("template rules keep English in the en locale", () => {
    i18n.setLocale("en");
    expect(tStepLabel("Install npm dependencies (frontend)")).toBe(
      "Install npm dependencies (frontend)",
    );
    expect(tStepLabel("Check go toolchain")).toBe("Check go toolchain");
    expect(tStepLabel("Create src/index.ts")).toBe("Create src/index.ts");
  });

  it("returns the original string for unknown labels and empty input", () => {
    i18n.setLocale("ru");
    expect(tStepLabel("Some totally new step")).toBe("Some totally new step");
    expect(tStepLabel("")).toBe("");
  });

  it("prefers the exact dictionary entry over the generic template", () => {
    // «Create MAUI app» в словаре переводится по-своему, а не через
    // generic «Создать приложение {fw}» — правила точного совпадения майорят.
    i18n.setLocale("ru");
    expect(tStepLabel("Create MAUI app")).toBe("Создать приложение MAUI");
    expect(tStepLabel("Create Flask app")).toBe("Создать приложение Flask");
  });
});