## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## Release
### 1. Create release branch from main
`git checkout -b release/vx.x.x`

### 2. Update versions in all files
>Update src-tauri/Cargo.toml, package.json, tauri.conf.json

### 3. Commit version changes
```sh
git add .
git commit -m "chore: bump version to vx.x.x"
```

### 4. Merge to main
```sh
git checkout main
git merge release/vx.x.x
```

### 5. Create and push tag
```sh
git tag vx.x.x
git push origin main --tags
```