export function replacePromptArgument(argument, prompt) {
  return argument.replaceAll('{prompt}', () => prompt);
}
