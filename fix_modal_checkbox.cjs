const fs = require('fs');
let file = 'src/lib/modules/toolchain/components/PlanReviewModal.svelte';
let content = fs.readFileSync(file, 'utf8');

// Change let confirmUnverified = $state(true); back to false
content = content.replace(/let confirmUnverified = \$state\(true\);/, 'let confirmUnverified = $state(false);');

// Add the checkbox UI back
let checkboxUI = `
      {#if unverifiedCount > 0}
        <label class="confirm">
          <input type="checkbox" bind:checked={confirmUnverified} />
          <span>
            Я понимаю, что {unverifiedCount} источник(ов) не имеют контрольных сумм, и подтверждаю установку.
          </span>
        </label>
      {/if}`;

content = content.replace(/{#if adminTools > 0}/, checkboxUI + '\n      {#if adminTools > 0}');

// Remove the filter that hid the warning from the list:
content = content.replace(/\{#each plan\.warnings\.filter\(w => planWarningInfo\(w\)\.kind !== "unverified_source"\) as w\}/, '{#each plan.warnings as w}');

fs.writeFileSync(file, content);
