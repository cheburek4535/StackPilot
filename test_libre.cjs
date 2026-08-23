async function translate(text, targetLang) {
    const res = await fetch('https://translate.argosopentech.com/translate', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ q: text, source: 'ru', target: targetLang, format: 'text' })
    });
    if (!res.ok) throw new Error(res.statusText);
    const data = await res.json();
    return data.translatedText;
}
translate("Привет мир", "en").then(console.log).catch(console.error);
