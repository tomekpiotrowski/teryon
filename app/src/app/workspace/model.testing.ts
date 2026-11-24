import { type Effort, modelLabel, type ModelOption } from './workspace.model';

/** A model catalog entry for chooser and command tests. */
export function modelOptionForTest(
  id: string,
  label = modelLabel(id),
  efforts: Effort[] = [],
  default_effort: Effort | null = null,
  is_default = false,
): ModelOption {
  return { id, label, efforts, default_effort, is_default };
}
