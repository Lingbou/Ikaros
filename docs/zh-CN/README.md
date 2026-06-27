# Ikaros 文档

[English docs](../en/README.md)

中文文档和 `docs/en/` 保持同名结构。功能变化后，两种语言的文档应同步更新。

本文档目录存放正式子系统文档。文档应当描述接口、调用上下文、持久化状态、失败语义和必须保持的约束，而不是记录“已经做了什么”。

写法参考 Linux kernel 子系统文档的实用风格：用清晰的边界和契约解释代码，必要的实现细节放在依赖它的接口附近，限制条件作为接口的一部分说明，后续计划统一放到根目录 [Roadmap](../../ROADMAP.md)。

## 阅读顺序

修改 runtime 行为前建议先读：

1. [架构](architecture.md)
2. [安全模型](safety-model.md)
3. [Harness 模型](harness-model.md)
4. [Agent loop](agent-loop.md)
5. [配置](configuration.md)

然后按实际修改的子系统继续阅读对应文档。

## 写法规则

- 先写给人看。开头先说明子系统负责什么，以及调用方应该怎么使用它。
- 入口文档保持短。JSON schema、协议行和完整命令输出放到 [API 参考](api-reference.md)
  或对应子系统文档。
- 优先使用短段落、分类命令列表和稳定标题，不要把生成式长段落直接放进正式文档。
- 后续计划统一放到根目录 [Roadmap](../../ROADMAP.md)，不要散落在子系统契约文档里。

## 核心文档

- [架构](architecture.md)
- [安全模型](safety-model.md)
- [Harness 模型](harness-model.md)
- [Agent loop](agent-loop.md)
- [配置](configuration.md)
- [API 参考](api-reference.md)
- [威胁模型](threat-model.md)

## Runtime 子系统

- [记忆模型](memory-model.md)
- [记忆 Provider](memory-providers.md)
- [Context 引擎](context-engine.md)
- [RAG 模型](rag-model.md)
- [模型 Provider](model-providers.md)
- [语音 Provider](voice-providers.md)
- [Persona 模型](persona-model.md)
- [Body 模型](body-model.md)
- [自动化模型](automation-model.md)
- [消息网关](message-gateway.md)
- [服务管理器模板](service-manager.md)

## 开发和运维

- [插件系统](plugin-system.md)
- [Self-modify](self-modify.md)
- [部署](deployment.md)
