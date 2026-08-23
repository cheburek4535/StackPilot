async function translate(text, targetLang) {
    // using a public lingva instance:
    const url = `https://lingva.ml/api/v1/ru/${targetLang}/${encodeURIComponent(text)}`;
    const res = await fetch(url);
    if (!res.ok) throw new Error(res.statusText);
    const data = await res.json();
    return data.translation;
}
translate("Привет мир", "en").then(console.log).catch(console.error);
