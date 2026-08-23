const fs = require('fs');
const path = require('path');
const { parse } = require('svelte/compiler');

const cyrillicRegex = /[А-Яа-яЁё]/;
let dictionary = {};
let keyCounter = 1;
let edits = []; // { file, patches: [{start, end, replacement}] }

function walk(dir) {
    let results = [];
    const list = fs.readdirSync(dir);
    list.forEach(file => {
        file = path.join(dir, file);
        const stat = fs.statSync(file);
        if (stat && stat.isDirectory()) {
            results = results.concat(walk(file));
        } else if (file.endsWith('.svelte')) {
            results.push(file);
        }
    });
    return results;
}

function processNode(node, patches, isAttribute = false) {
    if (!node) return;
    
    if (node.type === 'Element' || node.type === 'InlineComponent' || node.type === 'Slot' || node.type === 'Window' || node.type === 'Body' || node.type === 'Head') {
        if (node.attributes) {
            node.attributes.forEach(attr => {
                if (attr.type === 'Attribute' && attr.value && Array.isArray(attr.value)) {
                    // Check if it's a single Text node
                    if (attr.value.length === 1 && attr.value[0].type === 'Text' && cyrillicRegex.test(attr.value[0].data)) {
                        let text = attr.value[0].data.trim();
                        if (text) {
                            let key = Object.keys(dictionary).find(k => dictionary[k] === text);
                            if (!key) {
                                key = "ui_" + keyCounter++;
                                dictionary[key] = text;
                            }
                            patches.push({
                                start: attr.start,
                                end: attr.end,
                                replacement: `${attr.name}={i18n.t("${key}")}`
                            });
                        }
                    }
                }
            });
        }
        if (node.children) {
            node.children.forEach(child => processNode(child, patches));
        }
    } else if (node.type === 'Fragment') {
        if (node.children) {
            node.children.forEach(child => processNode(child, patches));
        }
    } else if (node.type === 'EachBlock' || node.type === 'IfBlock' || node.type === 'AwaitBlock' || node.type === 'KeyBlock') {
        if (node.children) node.children.forEach(c => processNode(c, patches));
        if (node.else) processNode(node.else, patches);
        if (node.then) processNode(node.then, patches);
        if (node.catch) processNode(node.catch, patches);
    } else if (node.type === 'Text') {
        if (cyrillicRegex.test(node.data)) {
            let text = node.data.trim();
            if (text.length > 0 && !text.startsWith('//') && !text.startsWith('/*')) {
                let key = Object.keys(dictionary).find(k => dictionary[k] === text);
                if (!key) {
                    key = "ui_" + keyCounter++;
                    dictionary[key] = text;
                }
                
                // Replace ONLY the trimmed part, keeping leading/trailing whitespace
                let leading = node.data.match(/^\s*/)[0];
                let trailing = node.data.match(/\s*$/)[0];
                
                patches.push({
                    start: node.start,
                    end: node.end,
                    replacement: `${leading}{i18n.t("${key}")}${trailing}`
                });
            }
        }
    }
}

const files = walk('src/routes').concat(walk('src/lib/components')).concat(walk('src/lib/modules/toolchain'));

files.forEach(file => {
    let content = fs.readFileSync(file, 'utf8');
    let ast;
    try {
        ast = parse(content);
    } catch (e) {
        console.error("Error parsing", file, e.message);
        return;
    }
    
    let patches = [];
    processNode(ast.html, patches);
    
    if (patches.length > 0) {
        // Add i18n import if not present
        if (!content.includes('import { i18n }')) {
            let scriptMatch = content.match(/<script[^>]*>/);
            if (scriptMatch) {
                patches.push({
                    start: scriptMatch.index + scriptMatch[0].length,
                    end: scriptMatch.index + scriptMatch[0].length,
                    replacement: `\n  import { i18n } from "$lib/core/i18n.svelte";`
                });
            } else {
                patches.push({
                    start: 0,
                    end: 0,
                    replacement: `<script>\n  import { i18n } from "$lib/core/i18n.svelte";\n</script>\n`
                });
            }
        }
        
        edits.push({ file, patches });
    }
});

// Apply patches (in reverse order of start to not invalidate offsets)
let totalPatches = 0;
edits.forEach(edit => {
    let content = fs.readFileSync(edit.file, 'utf8');
    edit.patches.sort((a, b) => b.start - a.start);
    edit.patches.forEach(p => {
        content = content.substring(0, p.start) + p.replacement + content.substring(p.end);
        totalPatches++;
    });
    fs.writeFileSync(edit.file, content);
});

fs.writeFileSync('extracted_dict.json', JSON.stringify(dictionary, null, 2));
console.log("Total files patched:", edits.length);
console.log("Total patches applied:", totalPatches);
console.log("Unique keys extracted:", Object.keys(dictionary).length);
