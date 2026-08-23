const fs = require('fs');

const path = 'src/routes/settings/+page.svelte';
let content = fs.readFileSync(path, 'utf8');

// I will just replace all occurrences of string literal keys that were missed 
// during extraction because they weren't matched by the Cyrillic regex since they 
// might have been already i18n keys or just english keys.

// But wait, the settings page ALREADY had i18n keys!
// Let me look at extracted_dict.json to see what it has.
