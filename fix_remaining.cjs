const fs = require('fs');

const dict = JSON.parse(fs.readFileSync('extracted_dict.json', 'utf8'));
const langs = ['en', 'es', 'zh', 'de', 'hi', 'pt', 'ja'];

for (const lang of langs) {
    const filePath = `src/lib/core/locales/${lang}.ts`;
    let content = fs.readFileSync(filePath, 'utf8');
    
    // Quick fix: remove any TODO: prefix that was left behind
    content = content.replace(/"TODO: (.*?)"/g, '"$1"');
    
    fs.writeFileSync(filePath, content);
}
console.log("Cleaned up any lingering TODOs.");
