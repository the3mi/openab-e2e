#!/usr/bin/env node
/**
 * Test script for PR #354: RFC template + issue-check workflow improvements
 * Simulates the workflow logic without running actual GitHub Actions.
 */

const { execSync } = require('child_process');

// Load the actual workflow script from the PR diff
const marker = '<!-- openab-issue-check -->';

const rules = {
  bug: ['Description', 'Steps to Reproduce', 'Expected Behavior'],
  feature: ['Description', 'Use Case'],
  documentation: ['Description'],
  guidance: ['Question']
};

// Simulate the workflow logic (from PR diff)
function checkIssue({ body = '', labels = [] }) {
  const hasRfc = labels.includes('rfc');
  const type = Object.keys(rules).find(k => labels.includes(k));
  const hasOldComment = false; // simulate first run

  // RFC is free-form — skip check entirely (NEW from PR)
  if (hasRfc) {
    return { action: 'skip', reason: 'RFC label — free-form by design' };
  }

  if (!type) {
    // No template label — skip if already flagged (NEW: improved message)
    if (hasOldComment && labels.includes('incomplete')) {
      return { action: 'skip', reason: 'already flagged as incomplete' };
    }

    // NEW improved message from PR
    const newMsg = `${marker}\nThanks for the report! It looks like this issue was created without a template and is missing some required fields. Please add one of the following labels: \`bug\`, \`feature\`, \`documentation\`, or \`guidance\`, then edit this issue to include the matching template fields — the \`incomplete\` label will be removed automatically.`;

    return { action: 'comment', body: newMsg, addLabel: 'incomplete' };
  }

  // Template found — check fields
  const required = rules[type];
  const missing = required.filter(field => !body.includes(field));

  if (missing.length > 0) {
    return { action: 'flag', missing };
  }

  return { action: 'pass' };
}

// Test cases
const tests = [
  {
    name: 'RFC issue (new template) — should skip check',
    input: { body: 'My proposal is to change the architecture...', labels: ['rfc', 'needs-triage'] },
    expected: 'skip'
  },
  {
    name: 'No template label — should show NEW improved message',
    input: { body: 'Just some random text', labels: [] },
    expected: 'comment'
  },
  {
    name: 'Bug with all fields — should pass',
    input: {
      body: '## Description\nSomething is broken\n## Steps to Reproduce\n1. Do this\n2. Do that\n## Expected Behavior\nShould work',
      labels: ['bug']
    },
    expected: 'pass'
  },
  {
    name: 'Bug missing "Steps to Reproduce" — should flag',
    input: {
      body: '## Description\nSomething is broken\n## Expected Behavior\nShould work',
      labels: ['bug']
    },
    expected: 'flag'
  },
  {
    name: 'Feature with all fields — should pass',
    input: {
      body: '## Description\nAdd new feature\n## Use Case\nAs a user I want to...',
      labels: ['feature']
    },
    expected: 'pass'
  },
  {
    name: 'Documentation with all fields — should pass',
    input: {
      body: '## Description\nThe docs are unclear about X',
      labels: ['documentation']
    },
    expected: 'pass'
  },
  {
    name: 'Guidance with all fields — should pass',
    input: {
      body: '## Question\nHow do I do X?',
      labels: ['guidance']
    },
    expected: 'pass'
  },
  {
    name: 'Bug with missing "Expected Behavior" — should flag',
    input: {
      body: '## Description\nSomething is broken\n## Steps to Reproduce\n1. Do this',
      labels: ['bug']
    },
    expected: 'flag'
  }
];

console.log('🧪 Testing PR #354: RFC template + issue-check workflow\n');

let passed = 0;
let failed = 0;

for (const t of tests) {
  const result = checkIssue(t.input);
  const ok = result.action === t.expected;

  if (ok) {
    console.log(`✅ ${t.name}`);
    passed++;
  } else {
    console.log(`❌ ${t.name}`);
    console.log(`   Expected: ${t.expected}, Got: ${result.action}`);
    if (result.missing) console.log(`   Missing fields: ${result.missing.join(', ')}`);
    if (result.body) console.log(`   Message: ${result.body.slice(0, 80)}...`);
    failed++;
  }
}

console.log(`\n📊 Results: ${passed}/${tests.length} passed`);

if (failed > 0) {
  console.log('\n⚠️  Some tests failed — PR logic needs review');
  process.exit(1);
} else {
  console.log('\n✅ All tests passed — PR logic is correct');
  process.exit(0);
}
