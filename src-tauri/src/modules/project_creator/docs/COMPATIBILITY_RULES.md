# Compatibility Rules & Decision Logic

This document explains how the tool compatibility system makes decisions.

## Philosophy

The system uses **graduated feedback** rather than binary allow/deny:

- **Error:** Blocks generation, must be fixed
- **Warning:** Allowed but flagged for user awareness
- **Auto-suggest:** Quietly adds dependencies when obvious

This mirrors real-world architecture: some choices are wrong, some are questionable, and some are just preferences.

## Validation Pipeline

When user selects technologies:

1. **Framework validation** (existing logic, unchanged)
   - Conflicts checked (Django vs FastAPI)
   - Main pairs validated
   - OS compatibility

2. **Tool dependency validation** (NEW)
   - Check `tool.requires[]`
   - Error if dependencies missing
   - Example: Alembic requires SQLAlchemy

3. **Tool language validation** (NEW)
   - Check `tool.for_languages[]`
   - Error if no language match
   - Example: npm requires JS/TS

4. **Responsibility overlap validation** (NEW)
   - Group tools by `responsibility`
   - Check `alternative_policy` for each pair
   - Generate error/warning/allow

5. **Framework-tool validation** (NEW)
   - Check `framework.tool_warnings`
   - Check `framework.tool_conflicts`
   - Generate warnings/errors

## Decision Matrix

### When to Use Error vs Warning

**Use ERROR when:**
- Technical impossibility (npm without JavaScript)
- Missing critical dependency (Alembic without SQLAlchemy)
- Mutual exclusion by design (two migration systems)
- Framework-tool hard conflict

**Use WARNING when:**
- Overlapping responsibilities (Django ORM + SQLAlchemy)
- Unconventional but valid choice (React + Vue together)
- Framework duplicates tool functionality
- Advanced use case possible but rare

**Use ALLOW when:**
- Complementary tools (pytest + coverage)
- Different responsibilities (linter + formatter)
- Independent tools (postgres + redis)

## Responsibility-Based Grouping

Tools are grouped by responsibility:
- `orm`: SQLAlchemy, Prisma, Django ORM
- `migrations`: Alembic, Django migrations, Prisma Migrate
- `testing`: pytest, unittest, Jest
- `package_manager`: npm, yarn, pnpm
- `database`: PostgreSQL, MySQL, SQLite

Within each group, `alternative_policy` determines behavior.

> Note: the examples above illustrate the responsibility categories. The current `wizard_tree.json` covers the Python ecosystem (sqlalchemy, alembic, prisma, drizzle, pytest, postgresql/mysql/sqlite, npm) — not every listed tool exists in the tree yet.

## Alternative Policies

### Policy: Exclusive

**Meaning:** Only one tool from this group allowed.

**Use for:**
- Migration systems (can't run Alembic + Django migrations)
- Package managers (can't use npm + yarn simultaneously)
- Databases (can't be PostgreSQL + MySQL primary)

**Example:**
```json
{
  "id": "alembic",
  "responsibility": "migrations",
  "alternatives": ["django"],
  "alternative_policy": "exclusive"
}
```

### Policy: Warn

**Meaning:** Can coexist, but user should be aware.

**Use for:**
- ORMs (can use Django ORM + SQLAlchemy, but why?)
- Overlapping frameworks (Flask + FastAPI in same project)

**Example:**
```json
{
  "id": "sqlalchemy",
  "responsibility": "orm",
  "alternatives": ["django"],
  "alternative_policy": "warn"
}
```

### Policy: Allow

**Meaning:** No restriction, perfectly fine.

**Use for:**
- Complementary tools (different responsibilities)
- Toolchain combinations (compiler + linter + formatter)

**Example:**
```json
{
  "id": "pytest",
  "responsibility": "testing",
  "alternatives": ["coverage"],
  "alternative_policy": "allow"
}
```

## Framework-Tool Interactions

Frameworks can declare:

1. **provides_capabilities:** What the framework includes
   - Example: Django provides ["orm", "migrations"]

2. **tool_warnings:** Tools that overlap with framework
   - Generates WARNING
   - Shows reason + recommendation
   - Example: Django warns about SQLAlchemy

3. **tool_conflicts:** Tools that are incompatible
   - Generates ERROR
   - Blocks generation
   - Use sparingly

## README Generation

When validation produces warnings, the generated README includes an "Architectural Decisions" section explaining:
- Why overlapping tools were selected
- Framework-tool interaction reasoning
- Recommendations for future maintainers

This documents architectural intent, not just technology choices.

## Edge Cases

### Django + SQLAlchemy + Alembic

**Validation result:** WARNING (not error)

**Reasoning:**
- Django ORM + SQLAlchemy is unusual but valid
- Some use cases: legacy DB, advanced SQL, multi-DB
- Warning informs user without blocking

**README output:**
- Explains Django has built-in ORM
- Notes SQLAlchemy was explicitly added
- Recommends documenting which handles what

### FastAPI + SQLAlchemy + Alembic

**Validation result:** PASS (no warnings)

**Reasoning:**
- FastAPI has no built-in ORM
- SQLAlchemy is standard choice
- Alembic is standard migrations for SQLAlchemy
- This is a clean, coherent stack

### Alembic without SQLAlchemy

**Validation result:** ERROR

**Reasoning:**
- Alembic is designed for SQLAlchemy
- Technical dependency, not preference
- Cannot function without SQLAlchemy

## Extending the System

To add new validation rules:

1. Add validation function in `validate.rs`
2. Call from `validate_stack()`
3. Use existing `StackIssue` type
4. Choose appropriate `StackSeverity`
5. Add tests in `validation_tests.rs`

To add new metadata fields:

1. Extend structs in `models.rs`
2. Use `#[serde(default)]` for backward compatibility
3. Update wizard_tree.json schema
4. Add validation logic if needed
5. Update this documentation