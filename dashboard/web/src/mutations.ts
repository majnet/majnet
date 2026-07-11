import { useMutation, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'

/**
 * A write action against the bot/recon APIs. `mutate(() => send(...))` runs the
 * thunk, toasts the server's message (or the error), invalidates the given
 * query keys, and optionally runs `onDone` (e.g. navigate).
 */
export function useApiMutation(opts: { invalidate?: readonly unknown[][]; onDone?: () => void } = {}) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (fn: () => Promise<string>) => fn(),
    onSuccess: (msg) => {
      toast.success(msg)
      opts.invalidate?.forEach((queryKey) => qc.invalidateQueries({ queryKey }))
      opts.onDone?.()
    },
    onError: (e: Error) => toast.error(e.message),
  })
}
