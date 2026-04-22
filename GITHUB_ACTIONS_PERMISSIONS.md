# GitHub Actions 权限问题修复

## 问题

```
Error: Resource not accessible by integration
```

这个错误发生在 `actions/create-release@v1` 步骤，原因是默认的 `GITHUB_TOKEN` 没有足够的权限来创建 Release。

## 解决方案

在 `.github/workflows/release.yml` 文件顶部添加权限声明：

```yaml
name: Release

on:
  push:
    tags:
      - 'v*'
  workflow_dispatch:

permissions:
  contents: write  # 添加这个权限

env:
  CARGO_TERM_COLOR: always
```

## 权限说明

- `contents: write` - 允许工作流创建 Release、上传资产和修改仓库内容

## 其他可能的解决方案

如果上述方法不起作用，还可以尝试：

### 方案 1: 使用更新的 Action

将 `actions/create-release@v1` 替换为 `softprops/action-gh-release@v1`：

```yaml
- name: Create Release
  uses: softprops/action-gh-release@v1
  with:
    draft: false
    prerelease: false
    generate_release_notes: true
  env:
    GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

### 方案 2: 检查仓库设置

1. 进入仓库的 **Settings** → **Actions** → **General**
2. 找到 **Workflow permissions** 部分
3. 选择 **Read and write permissions**
4. 勾选 **Allow GitHub Actions to create and approve pull requests**
5. 点击 **Save**

## 验证

修复后，重新推送标签：

```bash
# 删除远程标签
git push --delete origin v0.1.0

# 删除本地标签
git tag -d v0.1.0

# 提交修复
git add .github/workflows/release.yml
git commit -m "Fix: Add contents write permission for release workflow"
git push

# 重新创建并推送标签
git tag -a v0.1.0 -m "Release version 0.1.0"
git push origin v0.1.0
```

GitHub Actions 应该能够成功创建 Release。

## 参考

- [GitHub Actions Permissions](https://docs.github.com/en/actions/security-guides/automatic-token-authentication#permissions-for-the-github_token)
- [Workflow permissions](https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions#permissions)
