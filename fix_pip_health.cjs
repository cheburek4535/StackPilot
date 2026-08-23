const fs = require('fs');
let file = 'src-tauri/src/modules/toolchain/tools.json';
let content = fs.readFileSync(file, 'utf8');

// The pip health check is:
// "health_checks": [ { "label": "pip", "command": [ "pip", "--version" ] } ]
content = content.replace(
  /"health_checks": \[\s*\{\s*"label": "pip",\s*"command": \[\s*"pip",\s*"--version"\s*\]\s*\}\s*\]/m,
  `"health_checks": [\n      {\n        "label": "pip",\n        "command": [\n          "python",\n          "-m",\n          "pip",\n          "--version"\n        ]\n      }\n    ]`
);

fs.writeFileSync(file, content);
