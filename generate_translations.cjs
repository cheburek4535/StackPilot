const fs = require('fs');

const dict = JSON.parse(fs.readFileSync('extracted_dict.json', 'utf8'));
const keys = Object.keys(dict);
const langs = ['en', 'es', 'zh', 'de', 'hi', 'pt', 'ja'];
const SEPARATOR = ' ||| ';

async function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}

async function translateBatch(texts, targetLang) {
    const text = texts.join(SEPARATOR);
    const url = `https://api.mymemory.translated.net/get?q=${encodeURIComponent(text)}&langpair=ru|${targetLang}&de=bot@aistudio.build`;
    
    for (let attempt = 1; attempt <= 3; attempt++) {
        try {
            const res = await fetch(url);
            const data = await res.json();
            
            if (data.responseStatus !== 200) {
                console.warn(`Attempt ${attempt} failed:`, data.responseDetails);
                await sleep(1000);
                continue;
            }
            
            const translatedText = data.responseData.translatedText;
            const parts = translatedText.split(SEPARATOR).map(s => s.trim());
            
            // basic fallback if split fails
            if (parts.length !== texts.length) {
                console.warn(`Mismatch in batch split for ${targetLang}. Expected ${texts.length}, got ${parts.length}.`);
                // just return original or something
                return texts.map(t => "TODO: " + t);
            }
            return parts;
        } catch (e) {
            console.error(e);
            await sleep(1000);
        }
    }
    return texts.map(t => "TODO: " + t);
}

async function processTranslations() {
    for (const lang of langs) {
        console.log(`Translating to ${lang}...`);
        const translations = {};
        let currentBatchKeys = [];
        let currentBatchTexts = [];
        let currentLength = 0;
        
        for (let i = 0; i < keys.length; i++) {
            const key = keys[i];
            const text = dict[key];
            
            // Check if adding this text exceeds ~400 chars (safe margin for 500 max)
            if (currentLength + text.length + SEPARATOR.length > 350) {
                // translate current batch
                const res = await translateBatch(currentBatchTexts, lang);
                res.forEach((t, idx) => translations[currentBatchKeys[idx]] = t);
                
                await sleep(500); // rate limiting
                
                currentBatchKeys = [];
                currentBatchTexts = [];
                currentLength = 0;
            }
            
            currentBatchKeys.push(key);
            currentBatchTexts.push(text);
            currentLength += text.length + SEPARATOR.length;
        }
        
        if (currentBatchKeys.length > 0) {
            const res = await translateBatch(currentBatchTexts, lang);
            res.forEach((t, idx) => translations[currentBatchKeys[idx]] = t);
        }
        
        // Write to file
        let fileContent = 'export default {\n';
        for (const k in translations) {
            fileContent += `  "${k}": ${JSON.stringify(translations[k])},\n`;
        }
        fileContent += '};\n';
        
        fs.writeFileSync(`src/lib/core/locales/${lang}.ts`, fileContent);
        console.log(`Saved src/lib/core/locales/${lang}.ts`);
    }
    
    // Write Russian base
    let ruContent = 'export default {\n';
    for (const k in dict) {
        ruContent += `  "${k}": ${JSON.stringify(dict[k])},\n`;
    }
    ruContent += '};\n';
    fs.writeFileSync(`src/lib/core/locales/ru.ts`, ruContent);
}

processTranslations().then(() => console.log("Done!")).catch(console.error);
