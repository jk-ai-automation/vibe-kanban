-- 需求编号改为按项目发号：编号游标与 simple_id 前缀落在 local_projects 上。
-- 好处：删除末条需求后编号不复用；项目改名后已有与新建需求的前缀保持一致。
-- 同时补齐排序/关联索引，并把历史时间戳规整成 RFC3339（与 sqlx 写入格式一致，
-- 这样同一行里由 DEFAULT 写入的 created_at 和由 Rust 绑定的 start_date 可直接按字符串比较）。

ALTER TABLE local_projects ADD COLUMN simple_id_prefix  TEXT    NOT NULL DEFAULT 'ISS';
ALTER TABLE local_projects ADD COLUMN next_issue_number INTEGER NOT NULL DEFAULT 1;

-- 编号游标回填：取现有需求的 MAX(issue_number) + 1，保证不会重发已用过的编号。
UPDATE local_projects
SET next_issue_number = COALESCE(
        (SELECT MAX(i.issue_number) FROM issues i WHERE i.project_id = local_projects.id),
        0
    ) + 1;

-- 前缀回填（一）：优先沿用现有需求 simple_id 的前缀，避免历史需求与新需求前缀不一致。
UPDATE local_projects
SET simple_id_prefix = (
        SELECT substr(i.simple_id, 1, instr(i.simple_id, '-') - 1)
        FROM issues i
        WHERE i.project_id = local_projects.id
          AND instr(i.simple_id, '-') > 1
        ORDER BY i.issue_number ASC
        LIMIT 1
    )
WHERE EXISTS (
        SELECT 1 FROM issues i
        WHERE i.project_id = local_projects.id
          AND instr(i.simple_id, '-') > 1
    );

-- 前缀回填（二）：没有需求的老项目按项目名取首字母，规则与
-- LocalProjects::simple_id_prefix 一致（最多 3 个 ASCII 首字母，大写，取不到用 ISS）。
CREATE TEMP TABLE _issue_prefix_backfill AS
WITH RECURSIVE initials(id, rest, acc) AS (
    SELECT id, ltrim(name) || ' ', '' FROM local_projects
    UNION ALL
    SELECT id,
           ltrim(substr(rest, instr(rest, ' ') + 1)),
           CASE
               WHEN length(acc) >= 3 THEN acc
               WHEN upper(substr(rest, 1, 1)) BETWEEN 'A' AND 'Z'
                   THEN acc || upper(substr(rest, 1, 1))
               ELSE acc
           END
    FROM initials
    WHERE rest <> ''
)
SELECT id, acc FROM initials WHERE rest = '';

UPDATE local_projects
SET simple_id_prefix = COALESCE(
        NULLIF((SELECT acc FROM _issue_prefix_backfill b WHERE b.id = local_projects.id), ''),
        'ISS'
    )
WHERE NOT EXISTS (SELECT 1 FROM issues i WHERE i.project_id = local_projects.id);

DROP TABLE _issue_prefix_backfill;

-- 快照按 (project_id, sort_order) 排序；标签反查与评论树查询各补一个索引。
CREATE INDEX IF NOT EXISTS idx_issues_project_sort   ON issues(project_id, sort_order);
CREATE INDEX IF NOT EXISTS idx_issue_tags_tag        ON issue_tags(tag_id);
CREATE INDEX IF NOT EXISTS idx_issue_comments_parent ON issue_comments(parent_id);

-- 历史时间戳规整：datetime('now','subsec') 写出的是 "YYYY-MM-DD HH:MM:SS.SSS"，
-- 而 sqlx 绑定 DateTime<Utc> 写出的是 RFC3339。两种格式混在一列里无法按字符串比较，
-- 这里把前者统一成后者（instr 区分大小写，LIKE 不区分，故用 instr）。
UPDATE local_projects SET created_at = replace(created_at, ' ', 'T') || '+00:00' WHERE instr(created_at, 'T') = 0;
UPDATE local_projects SET updated_at = replace(updated_at, ' ', 'T') || '+00:00' WHERE instr(updated_at, 'T') = 0;
UPDATE project_statuses SET created_at = replace(created_at, ' ', 'T') || '+00:00' WHERE instr(created_at, 'T') = 0;
UPDATE issues SET created_at = replace(created_at, ' ', 'T') || '+00:00' WHERE instr(created_at, 'T') = 0;
UPDATE issues SET updated_at = replace(updated_at, ' ', 'T') || '+00:00' WHERE instr(updated_at, 'T') = 0;
UPDATE issue_comments SET created_at = replace(created_at, ' ', 'T') || '+00:00' WHERE instr(created_at, 'T') = 0;
UPDATE issue_comments SET updated_at = replace(updated_at, ' ', 'T') || '+00:00' WHERE instr(updated_at, 'T') = 0;
