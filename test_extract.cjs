const fs = require('fs');
const content = fs.readFileSync('src/routes/devlauncher/analyze/+page.svelte', 'utf8');

let m;
const re = /(["'`])([^"'`]*[А-Яа-яЁё][^"'`]*)\1/g;
while ((m = re.exec(content)) !== null) {
    console.log("STRING:", m[2]);
}
