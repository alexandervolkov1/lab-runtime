"""Validate the shared user-package allowlist and its offline documentation."""
import json
import re
from urllib.parse import unquote, urlsplit


def user_files(root):
    files = json.loads((root / 'scripts/user-package-files.json').read_text())
    if not files or len(set(files)) != len(files):
        raise RuntimeError('Empty or duplicate user package inventory')
    for name in files:
        if name not in {'README.md', 'LICENSE', 'examples/runtime.minimal.toml', 'examples/runtime.virtual.toml'} and not re.fullmatch(r'docs/(?:api/|reference/)?[a-z-]+\.md|docs/README\.md|docs/api/README\.md', name):
            raise RuntimeError(f'Invalid user package input: {name}')
        if not (root / name).is_file():
            raise RuntimeError(f'Missing user package input: {name}')
    required = {'README.md', 'LICENSE', 'docs/README.md', 'docs/api/README.md',
                'examples/runtime.minimal.toml', 'examples/runtime.virtual.toml'}
    if not required <= set(files):
        raise RuntimeError('User package is missing required Runtime documents/examples')
    return files


def headings(text):
    result, counts = set(), {}
    for heading in re.findall(r'^\s{0,3}#{1,6}\s+(.+?)\s*#*\s*$', text, re.M):
        heading = re.sub(r'\[([^\]]+)\]\([^)]+\)', r'\1', heading).replace('`', '').lower()
        slug = re.sub(r'\s+', '-', re.sub(r'[^\w\s-]', '', heading).strip())
        count = counts.get(slug, 0)
        counts[slug] = count + 1
        result.add(slug if not count else f'{slug}-{count}')
    return result


def validate_user_files(root, files):
    """Check package-relative links, headings, audience, file bounds and secret patterns."""
    root = root.resolve()
    for directory in ('clients', 'ai', 'docs/developer'):
        if (root / directory).exists():
            raise RuntimeError(f'Developer-only directory in user package: {directory}')
    examples = {p.relative_to(root).as_posix() for p in (root / 'examples').rglob('*') if p.is_file()}
    if examples != {'examples/runtime.minimal.toml', 'examples/runtime.virtual.toml'}:
        raise RuntimeError('User package must contain only the two safe virtual examples')
    for name in files:
        path = root / name
        text = path.read_text(encoding='utf-8-sig')
        if re.search(r'-----BEGIN (RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----|'
                     r'gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}|AKIA[A-Z0-9]{16}', text):
            raise RuntimeError(f'Potential secret in user package: {name}')
        if path.suffix != '.md':
            continue
        for target in re.findall(r'(?<!!)\[[^\]]*\]\(([^)]+)\)', text):
            link = urlsplit(target.strip('<>'))
            if link.scheme in ('http', 'https', 'mailto'):
                continue
            resolved = (path.parent / unquote(link.path)).resolve() if link.path else path
            if not resolved.is_relative_to(root) or not resolved.is_file():
                raise RuntimeError(f'Broken/escaping package link: {name} -> {target}')
            if link.fragment and (resolved.suffix != '.md' or
                                  unquote(link.fragment) not in headings(resolved.read_text(encoding='utf-8-sig'))):
                raise RuntimeError(f'Broken package heading: {name} -> {target}')
