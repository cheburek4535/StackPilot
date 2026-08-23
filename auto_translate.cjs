const fs = require('fs');

const dict = JSON.parse(fs.readFileSync('extracted_dict.json', 'utf8'));
const keys = Object.keys(dict);
const langs = ['en', 'es', 'zh', 'de', 'hi', 'pt', 'ja'];

async function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}

async function translateSingle(text, targetLang) {
    const url = `https://api.mymemory.translated.net/get?q=${encodeURIComponent(text)}&langpair=ru|${targetLang}&de=bot2@aistudio.build`;
    for (let attempt = 1; attempt <= 3; attempt++) {
        try {
            const res = await fetch(url);
            const data = await res.json();
            if (data.responseStatus === 200) {
                return data.responseData.translatedText;
            }
        } catch(e) {}
        await sleep(500);
    }
    return "TODO: " + text;
}

async function fixTodos() {
    for (const lang of langs) {
        const filePath = `src/lib/core/locales/${lang}.ts`;
        const content = fs.readFileSync(filePath, 'utf8');
        
        let out = "export default {\n";
        for (const k of keys) {
            let match = content.match(new RegExp(`"${k}":\\s*"(.*?)"`));
            let val = dict[k];
            if (match && !match[1].startsWith("TODO:")) {
                val = match[1];
            } else {
                console.log(`Fixing ${k} for ${lang}...`);
                val = await translateSingle(dict[k], lang);
                await sleep(200);
            }
            out += `  "${k}": ${JSON.stringify(val)},\n`;
        }
        out += "};\n";
        fs.writeFileSync(filePath, out);
    }
}

fixTodos().then(() => console.log("FIXED TODOS!")).catch(console.error);
