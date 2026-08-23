async function translate(text, targetLang) {
    const url = `https://api.mymemory.translated.net/get?q=${encodeURIComponent(text)}&langpair=ru|${targetLang}`;
    const res = await fetch(url);
    const data = await res.json();
    return data.responseData.translatedText;
}
translate("Привет мир", "en").then(console.log);
