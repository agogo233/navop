-- 连接级 AI 访问策略（issue #437）：策略属于连接内容，随连接一起云同步、一起导入导出。
-- 缺省值保持 NULL，读取时按「允许」处理，旧记录无需回填。
ALTER TABLE connections ADD COLUMN ai_access_level TEXT;
