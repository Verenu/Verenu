export function requiredCiRulesetPayload(current, contexts) {
  const rules = (current?.rules || []).filter(rule => rule.type !== 'required_status_checks');
  rules.push({
    type: 'required_status_checks',
    parameters: {
      strict_required_status_checks_policy: true,
      required_status_checks: contexts.map(context => ({ context, integration_id: 15368 })),
    },
  });
  return {
    name: 'Verenu required CI',
    target: 'branch',
    enforcement: 'active',
    conditions: { ref_name: { include: ['refs/heads/master'], exclude: [] } },
    // Required CI applies to every actor; never add a bypass exemption.
    bypass_actors: [],
    rules,
  };
}
