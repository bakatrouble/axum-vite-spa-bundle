// Writes a `.gz` sibling next to every text asset in `dist/`, so the example
// demonstrates the precompressed variants `serve_embedded` now negotiates.
import { gzipSync } from 'node:zlib';
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { extname, join } from 'node:path';

const COMPRESSIBLE = new Set(['.js', '.css', '.html', '.json', '.svg', '.map', '.txt']);

function walk(dir) {
    for (const entry of readdirSync(dir)) {
        const path = join(dir, entry);
        if (statSync(path).isDirectory()) {
            walk(path);
            continue;
        }
        if (!COMPRESSIBLE.has(extname(path))) {
            continue;
        }
        const source = readFileSync(path);
        const compressed = gzipSync(source, { level: 9 });
        if (compressed.length < source.length) {
            writeFileSync(`${path}.gz`, compressed);
        }
    }
}

walk('dist');
