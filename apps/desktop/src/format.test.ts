import { describe, expect, it } from "vitest";
import {
  adapterMessageLabel,
  diagnosticLabel,
  evidenceSummaryLabel,
  formatDuration,
  installStateLabel,
  stateMeta,
} from "./format";

describe("format", () => {
  it("uses business-readable Chinese states", () => {
    expect(stateMeta.WAITING_USER.label).toBe("等待人工");
    expect(stateMeta.UNKNOWN.label).toBe("状态不明");
  });

  it("formats duration without inventing unknown values", () => {
    expect(formatDuration()).toBe("未知");
    expect(formatDuration(65_000)).toBe("1 分 5 秒");
  });

  it("translates technical states into business-readable Chinese", () => {
    expect(installStateLabel("INSTALLED_NOT_RUNNING")).toBe("已安装，未运行");
    expect(adapterMessageLabel("no active process detected")).toBe("当前未检测到运行中的进程");
    expect(diagnosticLabel("NOT_CONFIGURED")).toBe("未配置");
    expect(evidenceSummaryLabel("RESULT event with E2 evidence")).toBe("产生结果，证据等级 E2");
  });
});
