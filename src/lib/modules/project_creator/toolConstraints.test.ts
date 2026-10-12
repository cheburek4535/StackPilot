import { describe, expect, it } from "vitest";
import en from "../../core/locales/en";
import ru from "../../core/locales/ru";
import { validateStack } from "./rules";
import type { WizardTreeData } from "./types";
import rawTree from "../../../../src-tauri/src/modules/project_creator/knowledge/wizard_tree.json";

const tree = rawTree as unknown as WizardTreeData;

describe("Tool constraints and mutual exclusion validation", () => {
  it("rejects picking both gradle and maven on the same project", () => {
    const issues = validateStack(
      tree,
      "rest-api",
      ["java"],
      [],
      ["spring-boot"],
      ["gradle", "maven"],
      "windows"
    );

    const hasConflict = issues.some(
      (i) =>
        i.severity === "Error" &&
        (i.message_key === "stack.tool.exclusive_alternatives" ||
          i.message_key === "stack.tool.conflict")
    );
    expect(hasConflict).toBe(true);
  });

  it("rejects picking both postgresql and mysql", () => {
    const issues = validateStack(
      tree,
      "rest-api",
      ["typescript"],
      [],
      ["express"],
      ["postgresql", "mysql"],
      "windows"
    );

    const hasConflict = issues.some(
      (i) =>
        i.severity === "Error" &&
        (i.message_key === "stack.tool.exclusive_alternatives" ||
          i.message_key === "stack.tool.conflict")
    );
    expect(hasConflict).toBe(true);
  });

  it("rejects picking both prisma and drizzle", () => {
    const issues = validateStack(
      tree,
      "rest-api",
      ["typescript"],
      [],
      ["express"],
      ["prisma", "drizzle"],
      "windows"
    );

    const hasConflict = issues.some(
      (i) =>
        i.severity === "Error" &&
        (i.message_key === "stack.tool.exclusive_alternatives" ||
          i.message_key === "stack.tool.conflict")
    );
    expect(hasConflict).toBe(true);
  });

  it("permits picking a valid non-conflicting stack", () => {
    const issues = validateStack(
      tree,
      "rest-api",
      ["java"],
      [],
      ["spring-boot"],
      ["gradle", "postgresql"],
      "windows"
    );

    const errors = issues.filter((i) => i.severity === "Error");
    expect(errors).toHaveLength(0);
  });
});

describe("New UI locales integrity", () => {
  const newFrameworks = [
    "angular", "vite", "rails", "astro", "hono", "actix_web",
    "echo", "fiber", "blazor", "remix", "adonisjs"
  ];
  const newTools = ["angular_cli", "biome", "vitest", "gcc"];
  const newToolchain = ["gcc", "ruby", "angular_cli", "biome", "vitest"];

  it("contains Russian descriptions for all new frameworks and tools", () => {
    for (const fw of newFrameworks) {
      const key = `wizard.fw.${fw}.desc`;
      expect(ru[key as keyof typeof ru], `Missing RU description for ${key}`).toBeTruthy();
    }
    for (const tool of newTools) {
      const key = `wizard.tool.${tool}.desc`;
      expect(ru[key as keyof typeof ru], `Missing RU description for ${key}`).toBeTruthy();
    }
    for (const tool of newToolchain) {
      const descKey = `tool.${tool}.desc`;
      const notesKey = `tool.${tool}.notes`;
      expect(ru[descKey as keyof typeof ru], `Missing RU description for ${descKey}`).toBeTruthy();
      expect(ru[notesKey as keyof typeof ru], `Missing RU notes for ${notesKey}`).toBeTruthy();
    }
  });

  it("contains English descriptions for all new frameworks and tools", () => {
    for (const fw of newFrameworks) {
      const key = `wizard.fw.${fw}.desc`;
      expect(en[key as keyof typeof en], `Missing EN description for ${key}`).toBeTruthy();
    }
    for (const tool of newTools) {
      const key = `wizard.tool.${tool}.desc`;
      expect(en[key as keyof typeof en], `Missing EN description for ${key}`).toBeTruthy();
    }
    for (const tool of newToolchain) {
      const descKey = `tool.${tool}.desc`;
      const notesKey = `tool.${tool}.notes`;
      expect(en[descKey as keyof typeof en], `Missing EN description for ${descKey}`).toBeTruthy();
      expect(en[notesKey as keyof typeof en], `Missing EN notes for ${notesKey}`).toBeTruthy();
    }
  });

  it("defines tool conflict localization strings", () => {
    expect(ru["create.tool_unavailable_with"]).toContain("{name}");
    expect(en["create.tool_unavailable_with"]).toContain("{name}");
    expect(ru["create.tool_requires_lang"]).toContain("{name}");
    expect(en["create.tool_requires_lang"]).toContain("{name}");
    expect(ru["stack.tool.conflict"]).toContain("{a}");
    expect(en["stack.tool.conflict"]).toContain("{a}");
  });
});
