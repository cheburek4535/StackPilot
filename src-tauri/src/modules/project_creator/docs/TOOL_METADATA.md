# Tool & Framework Metadata Schema

This document describes the metadata schema used in `wizard_tree.json` for defining tool compatibility rules.

## Overview

The Project Creator uses a responsibility-based compatibility model. Tools and frameworks declare:
- What responsibilities they handle (e.g., "orm", "migrations")
- What capabilities they provide or require
- Which tools are alternatives
- How strictly alternatives are enforced

This metadata powers validation that catches:
- Missing dependencies (Alembic without SQLAlchemy)
- Language mismatches (npm without JavaScript)
- Responsibility overlaps (Django ORM + SQLAlchemy)
- Framework-tool incompatibilities (Django + Prisma)

## FrameworkDef Schema

### New Fields (added in tool compatibility system):

```json
{
  "provides_capabilities": ["string"],
  "tool_warnings": {
    "tool_id": {
      "reason": "string",
      "recommendation": "string"
    }
  },
  "tool_conflicts": ["string"]
}
```

**provides_capabilities** (optional, array of strings)
- Capabilities this framework provides (e.g., "orm", "migrations", "admin")
- Used to detect when tools duplicate framework functionality
- Example: Django provides ["orm", "migrations", "admin", "auth"]

**tool_warnings** (optional, object)
- Tools that should generate warnings when selected with this framework
- Key: tool_id, Value: {reason, recommendation}
- Example: Django warns about "sqlalchemy" because Django has built-in ORM

**tool_conflicts** (optional, array of strings)
- Tools that are completely incompatible (blocks generation)
- Use sparingly - prefer tool_warnings for soft incompatibilities

## ToolDef Schema

### New Fields:

```json
{
  "responsibility": "string or null",
  "alternatives": ["string"],
  "alternative_policy": "allow" | "warn" | "exclusive",
  "for_frameworks": ["string"],
  "requires_capabilities": ["string"]
}
```

**responsibility** (optional, string)
- Architectural responsibility category
- Standard values: "orm", "migrations", "testing", "linting", "package_manager", "database"
- Used for grouping and overlap detection
- Tools with same responsibility trigger policy checks

**alternatives** (optional, array of strings)
- IDs of tools/frameworks serving same/similar responsibility
- Used with alternative_policy to control coexistence
- Example: SQLAlchemy alternatives: ["prisma", "django"]

**alternative_policy** (optional, enum)
- `"allow"` (default): Can coexist, no restriction
- `"warn"`: Show warning about overlap
- `"exclusive"`: Mutual exclusion (error, blocks generation)

**for_frameworks** (optional, array of strings)
- Framework IDs this tool is designed for
- Stronger filter than for_languages
- Example: Alembic is designed for SQLAlchemy-based projects

**requires_capabilities** (optional, array of strings)
- Capabilities required from framework/stack
- Example: hypothetical ORM plugin might require "database_url"

## Validation Rules

### 1. Dependency Enforcement
- `tool.requires[]` must all be selected
- Error severity (blocks generation)
- Example: Alembic requires SQLAlchemy

### 2. Language Compatibility
- `tool.for_languages[]` must match at least one selected language
- Empty array = universal tool (no restriction)
- Error severity
- Example: npm requires JavaScript or TypeScript

### 3. Responsibility Overlap
- Tools with same `responsibility` checked against `alternative_policy`
- Exclusive → Error
- Warn → Warning
- Allow → No message
- Example: SQLAlchemy + Prisma (both orm, exclusive) → Error

### 4. Framework-Tool Warnings
- `framework.tool_warnings[tool_id]` triggers warning
- `framework.tool_conflicts[tool_id]` triggers error
- Example: Django + SQLAlchemy → Warning (not error)

## Adding New Tools

When adding a new tool to wizard_tree.json:

1. **Identify responsibility:**
   - What architectural concern does it address?
   - Use existing categories when possible

2. **Find alternatives:**
   - What other tools solve the same problem?
   - Are they mutually exclusive or can they coexist?

3. **Set alternative_policy:**
   - Exclusive: migrations systems (Alembic vs Django migrations)
   - Warn: ORMs that could overlap (SQLAlchemy vs Django ORM)
   - Allow: complementary tools (pytest + coverage)

4. **Define dependencies:**
   - Does it require other tools? (add to `requires[]`)
   - What languages does it support? (add to `for_languages[]`)

5. **Check framework interactions:**
   - Does any framework provide this tool's functionality?
   - Should frameworks warn about this tool?
   - Update framework.tool_warnings if needed

## Adding New Frameworks

When adding a new framework:

1. **Declare provided capabilities:**
   - What built-in features does it have?
   - Example: Django provides ORM, migrations, admin, auth

2. **Configure tool warnings:**
   - Which tools duplicate framework features?
   - Provide clear reason + recommendation

3. **Identify tool conflicts:**
   - Any tools completely incompatible?
   - Use sparingly - prefer warnings

## Examples

### Example 1: ORM Tool (SQLAlchemy)

```json
{
  "id": "sqlalchemy",
  "label": "SQLAlchemy",
  "responsibility": "orm",
  "alternatives": ["prisma", "django"],
  "alternative_policy": "warn",
  "for_languages": ["python"],
  "requires": [],
  "for_frameworks": []
}
```

**Reasoning:**
- responsibility="orm" groups it with other ORMs
- alternative_policy="warn" allows Django+SQLAlchemy but warns user
- for_languages=["python"] enforces language requirement

### Example 2: Migration Tool (Alembic)

```json
{
  "id": "alembic",
  "label": "Alembic",
  "responsibility": "migrations",
  "alternatives": ["django", "prisma"],
  "alternative_policy": "exclusive",
  "for_languages": ["python"],
  "requires": ["sqlalchemy"],
  "for_frameworks": []
}
```

**Reasoning:**
- alternative_policy="exclusive" prevents mixing migration systems
- requires=["sqlalchemy"] enforces tight coupling
- alternatives include Django (which has built-in migrations)

### Example 3: Framework with Built-in ORM (Django)

```json
{
  "id": "django",
  "provides_capabilities": ["orm", "migrations", "admin", "auth"],
  "tool_warnings": {
    "sqlalchemy": {
      "reason": "Django includes a built-in ORM. Adding SQLAlchemy creates two ORMs.",
      "recommendation": "Use Django ORM unless you need advanced SQL features."
    }
  },
  "tool_conflicts": []
}
```

**Reasoning:**
- provides_capabilities declares Django's built-in features
- tool_warnings explains overlap without blocking
- tool_conflicts empty (SQLAlchemy is allowed, just warned)

> Note: `reason` and `recommendation` values in wizard_tree.json are localized UI strings; the texts shown above are illustrative.

## Testing Your Metadata

After modifying wizard_tree.json, run:

```bash
cargo test metadata_tests  # Validates IDs and references
cargo test validation_tests  # Tests validation logic
```

Common mistakes:
- Invalid tool/framework IDs in requires/alternatives/warnings
- Conflicting policies (exclusive + allow on same pair)
- Missing dependencies (tool requires another tool not in tree)