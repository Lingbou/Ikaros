import { useCallback, useSyncExternalStore } from "react";
import type { UiLanguagePreference } from "../shared/platform";

export type UiLanguage = UiLanguagePreference;
export type TranslationValues = Record<string, string | number>;

const english = {
  "app.conversation": "Conversation",

  "common.chat": "Chat",
  "common.unavailable": "Unavailable",
  "common.notAvailableYet": "Not available yet",
  "common.cancel": "Cancel",

  "titlebar.showSidebar": "Show sidebar",
  "titlebar.hideSidebar": "Hide sidebar",
  "titlebar.back": "Back",
  "titlebar.forward": "Forward",
  "titlebar.applicationMenu": "Application menu",
  "titlebar.file": "File",
  "titlebar.edit": "Edit",
  "titlebar.view": "View",
  "titlebar.help": "Help",
  "titlebar.menuUnavailable": "{menu} menu (not available yet)",
  "titlebar.minimize": "Minimize",
  "titlebar.maximize": "Maximize",
  "titlebar.close": "Close",

  "branch.main": "Main",
  "branch.number": "Branch {number}",

  "composer.placeholder": "Ask Ikaros to work on something",
  "composer.messageLabel": "Message Ikaros",
  "composer.addContext": "Add context",
  "composer.attachFile": "Attach file",
  "composer.addProjectFolder": "Add project folder",
  "composer.chooseTools": "Choose tools",
  "composer.sendMessage": "Send message",
  "composer.stopRun": "Stop",
  "composer.localModel": "Local model",
  "composer.accessPrompt": "How should Ikaros actions be approved?",
  "composer.access.ask.label": "Ask for approval",
  "composer.access.ask.shortLabel": "Ask for approval",
  "composer.access.ask.description":
    "Always ask to edit external files and use the internet.",
  "composer.access.safe.label": "Approve for me",
  "composer.access.safe.shortLabel": "Approve for me",
  "composer.access.safe.description":
    "Only ask for actions detected as potentially unsafe.",
  "composer.access.full.label": "Full access",
  "composer.access.full.shortLabel": "Full access",
  "composer.access.full.description":
    "Unrestricted access to the internet and any file on your computer.",

  "slash.label": "Slash commands",
  "slash.heading": "Commands",
  "slash.noMatches": "No matching commands",
  "slash.mock1.label": "Mock feature 1",
  "slash.mock1.description": "Summarize the current conversation",
  "slash.mock2.label": "Mock feature 2",
  "slash.mock2.description": "Create a structured plan",
  "slash.mock3.label": "Mock feature 3",
  "slash.mock3.description": "Inspect the available tools",

  "search.title": "Search chats",
  "search.close": "Close search",
  "search.noMatches": "No conversations match “{query}”.",
  "search.countOne": "1 conversation",
  "search.countMany": "{count} conversations",
  "search.hint": "Enter to open · Esc to close",

  "edit.title": "Edit into a new branch",
  "edit.description":
    "The original message and everything after it stay intact. Ikaros will create a new active branch from this point.",
  "edit.cancel": "Cancel edit",
  "edit.createBranch": "Create branch",

  "empty.heading": "What should we work on?",
  "empty.description": "Start a conversation to plan, explore, create, or get work done.",

  "events.conversationLog": "Conversation events",
  "turnNavigator.label": "Conversation turns",
  "turnNavigator.jumpTo": "Jump to turn {position} of {count}",
  "turnNavigator.position": "Turn {position} of {count}",
  "turnNavigator.untitled": "Conversation turn",
  "events.running": "Running",
  "events.complete": "Complete",
  "events.failed": "Failed",
  "events.interrupted": "Interrupted",
  "events.allowed": "Allowed",
  "events.denied": "Denied",
  "events.deny": "Deny",
  "events.allowOnce": "Allow once",
  "events.recovering": "Recovering",
  "events.recovered": "Recovered",
  "events.retryStep": "Retry step",
  "events.recoverSession": "Recover session",
  "events.artifactMetadata": "Markdown artifact · v{version} · Generated",
  "events.warning": "Warning",
  "events.error": "Error",
  "events.copyMessage": "Copy message",
  "events.editAndBranch": "Edit and branch",
  "events.copyResponse": "Copy response",
  "events.regenerateUnavailable": "Regenerate response (not available yet)",
  "events.file.created": "Created file",
  "events.file.modified": "Modified file",
  "events.file.deleted": "Deleted file",
  "events.file.renamed": "Renamed file",
  "events.runStopped.title": "Run stopped",
  "events.runStopped.description":
    "You stopped this run. The conversation and completed activity are preserved.",
  "events.branchCreatedFromEdit": "Branched from edited message",
  "events.tool.inspectArchiveMetadata": "Inspecting archive metadata",
  "events.tool.testArchiveExtraction": "Testing archive extraction",
  "events.tool.exportReportPackage": "Exporting report package",
  "events.result.archiveManifestVerified": "Archive manifest verified",
  "events.result.windowsReservedNames": "Two filenames cannot be extracted on Windows",
  "events.permission.moveReceiptsTitle": "Allow moving {count} receipt files?",
  "events.permission.moveReceiptsDescription":
    "The agent will create monthly folders and move matching PDFs. Originals remain recoverable from the Recycle Bin.",
  "events.workerDisconnected.title": "Local worker disconnected",
  "events.workerDisconnected.description":
    "The run stopped after the DOCX was written. Its checkpoint and generated files are still available.",
  "events.recoveringCheckpoint.title": "Recovering from checkpoint",
  "events.recoveringCheckpoint.description":
    "Reattached to the saved run and verified the existing DOCX before continuing.",
  "events.retryingExport.title": "Retrying export step",
  "events.retryingExport.description":
    "Started a fresh PDF export while preserving the completed DOCX.",
  "events.recoveryCompleted.title": "Recovery completed",
  "events.recoveryCompleted.description":
    "The saved run is healthy again and both output files passed validation.",
  "events.status.buildingBrief": "Building brief from {count} notes",
  "events.status.briefBuilt": "Brief built from {count} notes",
  "events.status.sourcesLinked": "Sources stay linked to the artifact.",
  "events.status.checkpointRestored": "Checkpoint restored",
  "events.status.exportStepRetried": "Export step retried",
  "events.status.outputsValidated": "Both output files passed validation.",

  "sidebar.workspaceNavigation": "Workspace navigation",
  "sidebar.currentWorkspace": "Current workspace: {name}",
  "sidebar.searchConversations": "Search",
  "sidebar.hideSidebar": "Hide sidebar",
  "sidebar.newChat": "New chat",
  "sidebar.projects": "Projects",
  "sidebar.noChats": "No chats",
  "sidebar.recents": "Recents",
  "sidebar.openProfileMenu": "Open profile menu",
  "sidebar.settings": "Settings",

  "settings.backToApp": "Back to app",
  "settings.navigation": "Settings navigation",
  "settings.personal": "Personal",
  "settings.general": "General",
  "settings.profile": "Profile",
  "settings.profileEdit": "Edit",
  "settings.profileEditTitle": "Edit profile",
  "settings.profileEditDescription": "Change the username shown in the app.",
  "settings.profileUsername": "Username",
  "settings.profileSave": "Save",
  "settings.profileLifetimeTokens": "Lifetime tokens",
  "settings.profilePeakTokens": "Peak tokens",
  "settings.profileLongestChat": "Longest chat",
  "settings.profileCurrentStreak": "Current streak",
  "settings.profileLongestStreak": "Longest streak",
  "settings.profileTokenActivity": "Token activity",
  "settings.profileDaily": "Daily",
  "settings.profileWeekly": "Weekly",
  "settings.profileCumulative": "Cumulative",
  "settings.profileActivityInsights": "Activity insights",
  "settings.profileFastMode": "Fast mode",
  "settings.profileMostUsedReasoning": "Most used reasoning",
  "settings.profileSkillsExplored": "Skills explored",
  "settings.profileTotalSkillsUsed": "Total skills used",
  "settings.profileTotalChats": "Total chats",
  "settings.profileMostUsedSkills": "Most used skills",
  "settings.profileSkillRuns": "{count} runs",
  "settings.appearance": "Appearance",
  "settings.searchPlaceholder": "Search settings...",
  "settings.theme": "Theme",
  "settings.system": "System",
  "settings.light": "Light",
  "settings.dark": "Dark",
  "settings.themeName": "{theme} theme",
  "settings.accent": "Accent",
  "settings.background": "Background",
  "settings.foreground": "Foreground",
  "settings.uiFont": "UI font",
  "settings.codeFont": "Code font",
  "settings.translucentSidebar": "Translucent sidebar",
  "settings.contrast": "Contrast",
  "settings.preferences": "Preferences",
  "settings.reduceMotion": "Reduce motion",
  "settings.reduceMotionDescription": "Reduce interface animations and transitions",
  "settings.chooseColor": "Choose {setting} color",
  "settings.chooseUiFont": "Choose UI font",
  "settings.chooseCodeFont": "Choose code font",
  "settings.font.system": "System default",
  "settings.font.inter": "Inter",
  "settings.font.segoUi": "Segoe UI",
  "settings.font.systemMono": "System monospace",
  "settings.font.cascadiaCode": "Cascadia Code",
  "settings.font.consolas": "Consolas",
  "settings.language": "Language",
  "settings.languageDescription": "Language for the app UI",
  "settings.languageMenu": "Choose UI language",
  "settings.english": "English",
  "settings.simplifiedChinese": "简体中文",
  "settings.saving": "Saving…",
  "settings.error": "Could not save settings.",
} as const;

export type TranslationKey = keyof typeof english;
export type Translate = (key: TranslationKey, values?: TranslationValues) => string;

const simplifiedChinese: Record<TranslationKey, string> = {
  "app.conversation": "对话",

  "common.chat": "对话",
  "common.unavailable": "暂不可用",
  "common.notAvailableYet": "暂未开放",
  "common.cancel": "取消",

  "titlebar.showSidebar": "显示边栏",
  "titlebar.hideSidebar": "隐藏边栏",
  "titlebar.back": "后退",
  "titlebar.forward": "前进",
  "titlebar.applicationMenu": "应用菜单",
  "titlebar.file": "文件",
  "titlebar.edit": "编辑",
  "titlebar.view": "视图",
  "titlebar.help": "帮助",
  "titlebar.menuUnavailable": "{menu}菜单（暂未开放）",
  "titlebar.minimize": "最小化",
  "titlebar.maximize": "最大化",
  "titlebar.close": "关闭",

  "branch.main": "主分支",
  "branch.number": "分支 {number}",

  "composer.placeholder": "告诉 Ikaros 你想做什么",
  "composer.messageLabel": "给 Ikaros 发送消息",
  "composer.addContext": "添加上下文",
  "composer.attachFile": "附加文件",
  "composer.addProjectFolder": "添加项目文件夹",
  "composer.chooseTools": "选择工具",
  "composer.sendMessage": "发送消息",
  "composer.stopRun": "停止",
  "composer.localModel": "本地模型",
  "composer.accessPrompt": "应如何批准 Ikaros 操作？",
  "composer.access.ask.label": "请求批准",
  "composer.access.ask.shortLabel": "请求批准",
  "composer.access.ask.description": "编辑外部文件和使用互联网时始终询问。",
  "composer.access.safe.label": "替我审批",
  "composer.access.safe.shortLabel": "替我审批",
  "composer.access.safe.description": "仅对检测到的风险操作请求批准。",
  "composer.access.full.label": "完全访问权限",
  "composer.access.full.shortLabel": "完全访问",
  "composer.access.full.description": "可不受限制地访问互联网和您电脑上的任何文件。",

  "slash.label": "斜杠命令",
  "slash.heading": "命令",
  "slash.noMatches": "没有匹配的命令",
  "slash.mock1.label": "模拟功能 1",
  "slash.mock1.description": "总结当前对话",
  "slash.mock2.label": "模拟功能 2",
  "slash.mock2.description": "创建结构化计划",
  "slash.mock3.label": "模拟功能 3",
  "slash.mock3.description": "检查可用工具",

  "search.title": "搜索聊天",
  "search.close": "关闭搜索",
  "search.noMatches": "没有与“{query}”匹配的对话。",
  "search.countOne": "1 个对话",
  "search.countMany": "{count} 个对话",
  "search.hint": "按 Enter 打开 · 按 Esc 关闭",

  "edit.title": "编辑并创建新分支",
  "edit.description": "原消息及其后的内容都会保留。Ikaros 将从这里创建并切换到一个新分支。",
  "edit.cancel": "取消编辑",
  "edit.createBranch": "创建分支",

  "empty.heading": "我们接下来做什么？",
  "empty.description": "开始一段对话，让 Ikaros 帮你规划、探索、创作或完成任务。",

  "events.conversationLog": "对话事件",
  "turnNavigator.label": "对话轮次",
  "turnNavigator.jumpTo": "跳转到第 {position} 轮，共 {count} 轮",
  "turnNavigator.position": "第 {position} 轮，共 {count} 轮",
  "turnNavigator.untitled": "对话轮次",
  "events.running": "运行中",
  "events.complete": "已完成",
  "events.failed": "失败",
  "events.interrupted": "已中断",
  "events.allowed": "已允许",
  "events.denied": "已拒绝",
  "events.deny": "拒绝",
  "events.allowOnce": "允许一次",
  "events.recovering": "恢复中",
  "events.recovered": "已恢复",
  "events.retryStep": "重试此步骤",
  "events.recoverSession": "恢复会话",
  "events.artifactMetadata": "Markdown 产物 · v{version} · 已生成",
  "events.warning": "警告",
  "events.error": "错误",
  "events.copyMessage": "复制消息",
  "events.editAndBranch": "编辑并创建分支",
  "events.copyResponse": "复制回复",
  "events.regenerateUnavailable": "重新生成回复（暂未开放）",
  "events.file.created": "已创建文件",
  "events.file.modified": "已修改文件",
  "events.file.deleted": "已删除文件",
  "events.file.renamed": "已重命名文件",
  "events.runStopped.title": "运行已停止",
  "events.runStopped.description": "你已停止此次运行。对话和已完成的操作均已保留。",
  "events.branchCreatedFromEdit": "已从编辑的消息创建分支",
  "events.tool.inspectArchiveMetadata": "正在检查压缩包元数据",
  "events.tool.testArchiveExtraction": "正在测试压缩包解压",
  "events.tool.exportReportPackage": "正在导出报告包",
  "events.result.archiveManifestVerified": "压缩包清单已验证",
  "events.result.windowsReservedNames": "有两个文件名无法在 Windows 上解压",
  "events.permission.moveReceiptsTitle": "允许移动 {count} 个收据文件吗？",
  "events.permission.moveReceiptsDescription":
    "Ikaros 将创建按月分类的文件夹并移动匹配的 PDF。原文件仍可从回收站恢复。",
  "events.workerDisconnected.title": "本地执行器已断开",
  "events.workerDisconnected.description":
    "DOCX 写入后运行停止。检查点和已生成的文件仍然可用。",
  "events.recoveringCheckpoint.title": "正在从检查点恢复",
  "events.recoveringCheckpoint.description":
    "已重新连接到保存的运行，并在继续之前验证了现有 DOCX。",
  "events.retryingExport.title": "正在重试导出步骤",
  "events.retryingExport.description":
    "在保留已完成 DOCX 的同时，开始重新导出 PDF。",
  "events.recoveryCompleted.title": "恢复完成",
  "events.recoveryCompleted.description":
    "保存的运行已恢复正常，两个输出文件均已通过验证。",
  "events.status.buildingBrief": "正在根据 {count} 条笔记生成简报",
  "events.status.briefBuilt": "已根据 {count} 条笔记生成简报",
  "events.status.sourcesLinked": "来源会继续与产物保持关联。",
  "events.status.checkpointRestored": "检查点已恢复",
  "events.status.exportStepRetried": "导出步骤已重试",
  "events.status.outputsValidated": "两个输出文件均已通过验证。",

  "sidebar.workspaceNavigation": "工作区导航",
  "sidebar.currentWorkspace": "当前工作区：{name}",
  "sidebar.searchConversations": "搜索",
  "sidebar.hideSidebar": "隐藏边栏",
  "sidebar.newChat": "新对话",
  "sidebar.projects": "项目",
  "sidebar.noChats": "没有聊天",
  "sidebar.recents": "最近",
  "sidebar.openProfileMenu": "打开个人资料菜单",
  "sidebar.settings": "设置",

  "settings.backToApp": "返回应用",
  "settings.navigation": "设置导航",
  "settings.personal": "个人",
  "settings.general": "常规",
  "settings.profile": "个人资料",
  "settings.profileEdit": "编辑",
  "settings.profileEditTitle": "编辑个人资料",
  "settings.profileEditDescription": "更改应用中显示的用户名。",
  "settings.profileUsername": "用户名",
  "settings.profileSave": "保存",
  "settings.profileLifetimeTokens": "累计 Token 数",
  "settings.profilePeakTokens": "峰值 Token 数",
  "settings.profileLongestChat": "最长聊天时长",
  "settings.profileCurrentStreak": "当前连续天数",
  "settings.profileLongestStreak": "最长连续天数",
  "settings.profileTokenActivity": "Token 活动",
  "settings.profileDaily": "每日",
  "settings.profileWeekly": "每周",
  "settings.profileCumulative": "累计",
  "settings.profileActivityInsights": "活动洞察",
  "settings.profileFastMode": "快速模式",
  "settings.profileMostUsedReasoning": "最常用的推理强度",
  "settings.profileSkillsExplored": "已探索的技能",
  "settings.profileTotalSkillsUsed": "使用的技能总数",
  "settings.profileTotalChats": "聊天总数",
  "settings.profileMostUsedSkills": "最常用的技能",
  "settings.profileSkillRuns": "运行 {count} 次",
  "settings.appearance": "外观",
  "settings.searchPlaceholder": "搜索设置...",
  "settings.theme": "主题",
  "settings.system": "跟随系统",
  "settings.light": "浅色",
  "settings.dark": "深色",
  "settings.themeName": "{theme}主题",
  "settings.accent": "强调色",
  "settings.background": "背景",
  "settings.foreground": "前景",
  "settings.uiFont": "界面字体",
  "settings.codeFont": "代码字体",
  "settings.translucentSidebar": "半透明侧边栏",
  "settings.contrast": "对比度",
  "settings.preferences": "偏好设置",
  "settings.reduceMotion": "减少动态效果",
  "settings.reduceMotionDescription": "减少界面动画和过渡效果",
  "settings.chooseColor": "选择{setting}颜色",
  "settings.chooseUiFont": "选择界面字体",
  "settings.chooseCodeFont": "选择代码字体",
  "settings.font.system": "系统默认",
  "settings.font.inter": "Inter",
  "settings.font.segoUi": "Segoe UI",
  "settings.font.systemMono": "系统等宽字体",
  "settings.font.cascadiaCode": "Cascadia Code",
  "settings.font.consolas": "Consolas",
  "settings.language": "语言",
  "settings.languageDescription": "应用 UI 语言",
  "settings.languageMenu": "选择 UI 语言",
  "settings.english": "English",
  "settings.simplifiedChinese": "简体中文",
  "settings.saving": "正在保存…",
  "settings.error": "无法保存设置。",
};

const catalogs: Record<UiLanguage, Record<TranslationKey, string>> = {
  en: english,
  "zh-CN": simplifiedChinese,
};

let currentLanguage: UiLanguage = "en";
const listeners = new Set<() => void>();

function interpolate(template: string, values?: TranslationValues): string {
  if (!values) return template;
  return template.replace(/\{([A-Za-z0-9_]+)\}/g, (match, name: string) =>
    Object.prototype.hasOwnProperty.call(values, name) ? String(values[name]) : match,
  );
}

function getUiLanguage(): UiLanguage {
  return currentLanguage;
}

export function translate(
  key: TranslationKey,
  values?: TranslationValues,
  language: UiLanguage = currentLanguage,
): string {
  return interpolate(catalogs[language][key] ?? english[key], values);
}

export function setUiLanguage(language: UiLanguage): void {
  const changed = currentLanguage !== language;
  currentLanguage = language;

  if (typeof document !== "undefined") {
    document.documentElement.lang = language;
  }

  if (changed) {
    listeners.forEach((listener) => listener());
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useTranslation(): { language: UiLanguage; t: Translate } {
  const language = useSyncExternalStore(subscribe, getUiLanguage, getUiLanguage);
  const t = useCallback<Translate>(
    (key, values) => translate(key, values, language),
    [language],
  );
  return { language, t };
}
