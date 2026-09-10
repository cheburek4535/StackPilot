// ============================================================
// Локализованная генерация README.md (i18n)
// ============================================================
// Весь текст документа берётся из i18n-базы (locales/en.ts, locales/ru.ts)
// через инжектированный переводчик `t`: модуль не знает, на каком языке его
// вызвали, и НЕ содержит ветвлений RU/EN. Язык выбирает вызывающая сторона,
// передавая переводчик нужной локали (см. i18n.translateIn). Структура
// документа зависит только от данных стека.

export type ReadmeTranslate = (
  key: string,
  vars?: Record<string, string | number>,
) => string;

/** Именованный элемент стека: id + i18n-ключ подписи (из wizard_tree). */
export type ReadmeNamedItem = {
  id: string;
  labelKey: string;
};

export type ReadmeArchitecture = "integrated" | "decoupled" | "unknown";

export type ReadmeFeatures = {
  docker: boolean;
  testing: boolean;
  git: boolean;
  vscode: boolean;
  ci: boolean;
};

export type ReadmeInput = {
  projectName: string;
  projectType: ReadmeNamedItem | null;
  backendLanguages: ReadmeNamedItem[];
  frontendLanguages: ReadmeNamedItem[];
  frameworks: ReadmeNamedItem[];
  tools: ReadmeNamedItem[];
  localInfraTools: ReadmeNamedItem[];
  architecture: ReadmeArchitecture;
  features: ReadmeFeatures;
};

/** Ключ документа README в i18n-базе. */
function docKey(key: string): string {
  return `create.readme.doc.${key}`;
}

function uniqueNamed(items: ReadmeNamedItem[]): ReadmeNamedItem[] {
  const seen = new Set<string>();
  const out: ReadmeNamedItem[] = [];
  for (const item of items) {
    if (seen.has(item.id)) continue;
    seen.add(item.id);
    out.push(item);
  }
  return out;
}

function nameList(items: ReadmeNamedItem[], t: ReadmeTranslate): string {
  return items.map((item) => t(item.labelKey)).join(", ");
}

function featureList(features: ReadmeFeatures, t: ReadmeTranslate): string[] {
  const flags: Array<[keyof ReadmeFeatures, string]> = [
    ["docker", "features.docker"],
    ["testing", "features.testing"],
    ["git", "features.git"],
    ["vscode", "features.vscode"],
    ["ci", "features.ci"],
  ];
  return flags.filter(([flag]) => features[flag]).map(([, key]) => t(docKey(key)));
}

function overviewBody(
  input: ReadmeInput,
  languages: ReadmeNamedItem[],
  t: ReadmeTranslate,
): string {
  const type = input.projectType
    ? t(input.projectType.labelKey)
    : t(docKey("overview.unknown_type"));
  const sentences: string[] = [
    t(docKey("overview.body"), {
      name: input.projectName.trim() || "project",
      type,
    }),
  ];
  if (languages.length > 0) {
    sentences.push(t(docKey("overview.languages"), { list: nameList(languages, t) }));
  }
  if (input.frameworks.length > 0) {
    sentences.push(
      t(docKey("overview.frameworks"), { list: nameList(input.frameworks, t) }),
    );
  }
  if (input.tools.length > 0) {
    sentences.push(t(docKey("overview.tools"), { list: nameList(input.tools, t) }));
  }
  return sentences.join(" ");
}

function stackBody(
  input: ReadmeInput,
  languages: ReadmeNamedItem[],
  t: ReadmeTranslate,
): string[] {
  const out: string[] = [];
  const groups: Array<[string, ReadmeNamedItem[]]> = [
    ["stack.languages", languages],
    ["stack.frameworks", input.frameworks],
    ["stack.tools", input.tools],
    ["stack.local_infra", input.localInfraTools],
  ];
  let hasAnything = false;
  for (const [key, items] of groups) {
    if (items.length === 0) continue;
    hasAnything = true;
    out.push(`**${t(docKey(key))}**`, "");
    for (const item of items) out.push(`- ${t(item.labelKey)}`);
    out.push("");
  }
  const features = featureList(input.features, t);
  if (features.length > 0) {
    hasAnything = true;
    out.push(`**${t(docKey("stack.features"))}**`, "");
    for (const feature of features) out.push(`- ${feature}`);
    out.push("");
  }
  if (!hasAnything) out.push(t(docKey("stack.empty")), "");
  return out;
}

function structureBody(input: ReadmeInput, t: ReadmeTranslate): string {
  const entries: string[] = [];
  if (input.backendLanguages.length > 0) {
    entries.push(t(docKey("structure.backend")));
  }
  if (input.frontendLanguages.length > 0) {
    entries.push(t(docKey("structure.frontend")));
  }
  entries.push(t(docKey("structure.readme")));
  if (input.tools.length > 0) entries.push(t(docKey("structure.env")));
  if (input.localInfraTools.length > 0) {
    entries.push(t(docKey("structure.local_infra")));
  }
  if (input.features.docker) {
    entries.push(t(docKey("structure.dockerfile")));
    entries.push(t(docKey("structure.docker")));
  }
  if (input.features.git) entries.push(t(docKey("structure.gitignore")));
  if (input.features.ci) entries.push(t(docKey("structure.ci")));

  const tree: string[] = [`${input.projectName.trim() || "project"}/`];
  entries.forEach((entry, index) => {
    const branch = index === entries.length - 1 ? "└──" : "├──";
    tree.push(`${branch} ${entry}`);
  });
  return ["```", ...tree, "```"].join("\n");
}

function gettingStartedBody(input: ReadmeInput, t: ReadmeTranslate): string[] {
  const steps: string[] = [];
  if (input.tools.length > 0) {
    steps.push(t(docKey("getting_started.env")));
  }
  steps.push(t(docKey("getting_started.install")));
  steps.push(t(docKey("getting_started.run")));
  if (input.features.docker) {
    steps.push(t(docKey("getting_started.docker")));
  }
  return steps.map((step, index) => `${index + 1}. ${step}`);
}

/** Собрать README.md. Детерминированно: одинаковый вход → одинаковый текст. */
export function buildReadme(input: ReadmeInput, t: ReadmeTranslate): string {
  const languages = uniqueNamed([...input.backendLanguages, ...input.frontendLanguages]);
  const lines: string[] = [];

  lines.push(`# ${input.projectName.trim() || "project"}`, "");
  lines.push(t(docKey("tagline")), "");

  lines.push(`## ${t(docKey("overview.title"))}`, "");
  lines.push(overviewBody(input, languages, t), "");

  lines.push(`## ${t(docKey("architecture.title"))}`, "");
  lines.push(t(docKey(`architecture.${input.architecture}`)), "");

  lines.push(`## ${t(docKey("stack.title"))}`, "");
  lines.push(...stackBody(input, languages, t));

  lines.push(`## ${t(docKey("structure.title"))}`, "");
  lines.push(structureBody(input, t), "");

  lines.push(`## ${t(docKey("getting_started.title"))}`, "");
  lines.push(...gettingStartedBody(input, t), "");

  if (input.tools.length > 0 || input.localInfraTools.length > 0) {
    lines.push(`## ${t(docKey("environment.title"))}`, "");
    lines.push(t(docKey("environment.body")), "");
    if (input.localInfraTools.length > 0) {
      lines.push(t(docKey("environment.local_infra")), "");
    }
  }

  if (input.features.docker) {
    lines.push(`## ${t(docKey("docker.title"))}`, "");
    lines.push(t(docKey("docker.body")), "");
  }

  if (input.features.testing) {
    lines.push(`## ${t(docKey("testing.title"))}`, "");
    lines.push(t(docKey("testing.body")), "");
  }

  lines.push(`## ${t(docKey("next_steps.title"))}`, "");
  lines.push(t(docKey("next_steps.body")), "");

  lines.push("---", "", `*${t(docKey("generated_by"))}*`);

  return lines.join("\n");
}
