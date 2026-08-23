const fs = require('fs');
const path = require('path');

let strings = new Set();

function walk(dir) {
    const list = fs.readdirSync(dir);
    list.forEach(file => {
        file = path.join(dir, file);
        const stat = fs.statSync(file);
        if (stat && stat.isDirectory()) {
            walk(file);
        } else if (file.endsWith('.svelte') || file.endsWith('.ts')) {
            const content = fs.readFileSync(file, 'utf8');
            // Very naive line-by-line extraction
            const lines = content.split('\n');
            lines.forEach(line => {
                if (line.trim().startsWith('//') || line.trim().startsWith('/*') || line.trim().startsWith('*')) return;
                
                // match anything that has Cyrillic
                let m = line.match(/([А-Яа-яЁё][А-Яа-яЁё0-9\s,.:!?()\-«»]*[А-Яа-яЁёa-zA-Z0-9])/g);
                if (m) {
                    m.forEach(s => {
                        if (s.trim().length > 1) {
                            strings.add(s.trim());
                        }
                    });
                }
            });
        }
    });
}

walk('src/routes');
walk('src/lib/modules/toolchain');
walk('src/lib/components');

fs.writeFileSync('all_cyrillic.txt', Array.from(strings).sort().join('\n'));
console.log("Unique strings:", strings.size);
