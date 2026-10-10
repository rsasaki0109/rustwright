import tempfile
from pathlib import Path
import unittest

from proc_memory import sample


class MemoryTests(unittest.TestCase):
    def test_descendants_and_missing_pss(self):
        with tempfile.TemporaryDirectory() as directory:
            proc = Path(directory)
            for pid, parent, rss, pss in ((1, 0, 100, 80), (2, 1, 200, 120), (3, 2, 300, 210), (4, 0, 999, 999)):
                folder = proc / str(pid)
                folder.mkdir()
                (folder / 'status').write_text(f'PPid:\t{parent}\nVmRSS:\t{rss} kB\nVmHWM:\t{rss+10} kB\n')
                (folder / 'smaps_rollup').write_text(f'Pss:\t{pss} kB\n')
            row = sample(1, proc)
            self.assertEqual((row['driver_rss_kib'], row['driver_pss_kib'], row['driver_hwm_kib']), (100, 80, 110))
            self.assertEqual((row['descendant_processes'], row['descendant_rss_kib'], row['descendant_pss_kib']), (2, 500, 330))
            (proc / '3/smaps_rollup').unlink()
            self.assertIsNone(sample(1, proc)['descendant_pss_kib'])
            self.assertEqual(sample(1, proc)['descendant_rss_kib'], 500)
            (proc / '1/status').unlink()
            self.assertIsNone(sample(1, proc))


if __name__ == '__main__':
    unittest.main()
