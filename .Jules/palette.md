## 2026-10-17 - Icon-only Button Accessibility

**Learning:** Missing aria-labels on icon-only buttons (like theme toggles and inline add buttons) is a recurring accessibility issue in server-rendered Rust templates. Title attributes alone are insufficient.
**Action:** Always explicitly check for and add aria-label attributes to empty buttons when modifying server-rendered templates in pkgs/id/src/web/templates.rs.
