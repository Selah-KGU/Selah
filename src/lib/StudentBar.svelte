<script lang="ts">
  import type { StudentInfo } from "./stores";

  interface Props {
    student: StudentInfo;
  }

  let { student }: Props = $props();
</script>

{#if student.name}
  <div class="student-bar">
    <span class="sid">{student.student_id}</span>
    <span class="sname">{student.name}</span>
    <span class="sfaculty">{student.faculty} {student.department}</span>
  </div>
{/if}

<style>
  .student-bar {
    display: flex;
    gap: 12px;
    align-items: center;
    padding: 10px 14px;
    margin-bottom: 20px;
    background: var(--bg-secondary);
    border-radius: 10px;
    font-size: 12px;
    animation: fade-in 0.3s ease;
  }
  .sid {
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
    font-family: "SF Mono", "Menlo", monospace;
    font-size: 11px;
  }
  .sname {
    color: var(--text-primary);
    font-weight: 600;
  }
  .sfaculty {
    color: var(--text-secondary);
  }
</style>
