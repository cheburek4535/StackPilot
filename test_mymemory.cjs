async function translateBatch(texts, targetLang) {
    const text = texts.join(' | ');
    const url = `https://api.mymemory.translated.net/get?q=${encodeURIComponent(text)}&langpair=ru|${targetLang}`;
    const res = await fetch(url);
    const data = await res.json();
    return data.responseData.translatedText.split(' | ');
}
translateBatch(["Привет", "Мир", "Как дела"], "en").then(console.log).catch(console.error);
