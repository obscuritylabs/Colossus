using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;

namespace Colossus.Release
{
    public static class EmbeddedAuthenticode
    {
        private const uint NoUi = 2; // WTD_UI_NONE
        private const uint FileChoice = 1; // WTD_CHOICE_FILE
        private const uint VerifyState = 1; // WTD_STATEACTION_VERIFY
        private const uint CloseState = 2; // WTD_STATEACTION_CLOSE
        private const uint VerifySpecific = 1; // WSS_VERIFY_SPECIFIC

        public static void Verify(string path)
        {
            // Keep the signed bytes open without allowing replacement or writes while
            // WinTrust verifies the file and we inspect that exact verification's state.
            using var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
            var pathPointer = IntPtr.Zero;
            var filePointer = IntPtr.Zero;
            var settingsPointer = IntPtr.Zero;
            try
            {
                pathPointer = Marshal.StringToHGlobalUni(path);
                var input = new TrustFile
                {
                    Size = (uint)Marshal.SizeOf<TrustFile>(),
                    Path = pathPointer,
                    Handle = file.SafeFileHandle.DangerousGetHandle(),
                };
                filePointer = Marshal.AllocHGlobal(Marshal.SizeOf<TrustFile>());
                Marshal.StructureToPtr(input, filePointer, false);

                // Azure places the release signature at index zero. A secondary
                // signature must not change which signer we accept for these bytes.
                var settings = new SignatureSettings
                {
                    Size = (uint)Marshal.SizeOf<SignatureSettings>(),
                    Flags = VerifySpecific,
                    Index = 0,
                };
                settingsPointer = Marshal.AllocHGlobal(Marshal.SizeOf<SignatureSettings>());
                Marshal.StructureToPtr(settings, settingsPointer, false);
                var trust = new TrustData
                {
                    Size = (uint)Marshal.SizeOf<TrustData>(),
                    UiChoice = NoUi,
                    UnionChoice = FileChoice,
                    File = filePointer,
                    StateAction = VerifyState,
                    SignatureSettings = settingsPointer,
                };
                var action = new Guid("00AAC56B-CD44-11D0-8CC2-00C04FC295EE"); // Authenticode
                var noWindow = new IntPtr(-1);
                try
                {
                    // Files in the CEF distribution can match local Windows catalogs.
                    // WTD_CHOICE_FILE verifies the embedded signature without searching them.
                    var status = WinVerifyTrust(noWindow, ref action, ref trust);
                    if (status != 0)
                        throw new CryptographicException(
                            $"Embedded Authenticode verification failed for '{path}' (0x{status:X8}).");
                    var provider = WTHelperProvDataFromStateData(trust.State);
                    if (provider == IntPtr.Zero)
                        throw new CryptographicException($"Embedded Authenticode verification state is missing for '{path}'.");
                    var signer = WTHelperGetProvSignerFromChain(provider, 0, false, 0);
                    using var certificate = ReadCertificate(signer, "publisher", path);
                    if (!HasExpectedPublisher(certificate))
                        throw new CryptographicException($"Embedded Authenticode publisher is not Obscurity Labs LLC for '{path}'.");

                    var timestamp = WTHelperGetProvSignerFromChain(provider, 0, true, 0);
                    using var timestampCertificate = ReadCertificate(timestamp, "timestamp", path);
                }
                finally
                {
                    if (trust.State != IntPtr.Zero)
                    {
                        trust.StateAction = CloseState;
                        WinVerifyTrust(noWindow, ref action, ref trust);
                    }
                }
            }
            finally
            {
                Marshal.FreeHGlobal(settingsPointer);
                Marshal.FreeHGlobal(filePointer);
                Marshal.FreeHGlobal(pathPointer);
            }
        }

        private static bool HasExpectedPublisher(X509Certificate2 certificate)
        {
            int commonNames = 0;
            foreach (var attribute in certificate.SubjectName.EnumerateRelativeDistinguishedNames())
            {
                if (attribute.HasMultipleElements)
                    return false;
                if (attribute.GetSingleElementType().Value != "2.5.4.3")
                    continue;
                if (attribute.GetSingleElementValue() != "Obscurity Labs LLC")
                    return false;
                commonNames++;
            }
            return commonNames == 1;
        }

        private static X509Certificate2 ReadCertificate(IntPtr signer, string purpose, string path)
        {
            if (signer != IntPtr.Zero && Marshal.PtrToStructure<ProviderSigner>(signer).Error == 0)
            {
                var chainLeaf = WTHelperGetProvCertFromChain(signer, 0);
                if (chainLeaf != IntPtr.Zero)
                {
                    var context = Marshal.PtrToStructure<ProviderCertificate>(chainLeaf).Context;
                    if (context != IntPtr.Zero)
                        return new X509Certificate2(context);
                }
            }
            throw new CryptographicException($"Trusted embedded Authenticode {purpose} is missing for '{path}'.");
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct TrustFile
        {
            public uint Size;
            public IntPtr Path;
            public IntPtr Handle;
            public IntPtr Subject;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct SignatureSettings
        {
            public uint Size;
            public uint Index;
            public uint Flags;
            public uint SecondaryCount;
            public uint VerifiedIndex;
            public IntPtr CryptoPolicy;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct TrustData
        {
            public uint Size;
            public IntPtr PolicyData;
            public IntPtr SipData;
            public uint UiChoice;
            public uint RevocationChecks;
            public uint UnionChoice;
            public IntPtr File;
            public uint StateAction;
            public IntPtr State;
            public IntPtr Url;
            public uint ProviderFlags;
            public uint UiContext;
            public IntPtr SignatureSettings;
        }

        // Prefixes of CRYPT_PROVIDER_SGNR and CRYPT_PROVIDER_CERT. The WinTrust
        // provider owns all memory reached through these pointers.
        [StructLayout(LayoutKind.Sequential)]
        private struct ProviderSigner
        {
            public uint Size;
            public System.Runtime.InteropServices.ComTypes.FILETIME VerifiedAt;
            public uint ChainLength;
            public IntPtr Chain;
            public uint SignerType;
            public IntPtr SignerInfo;
            public uint Error;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct ProviderCertificate
        {
            public uint Size;
            public IntPtr Context;
        }

        [DllImport("wintrust.dll", ExactSpelling = true)]
        private static extern int WinVerifyTrust(IntPtr window, ref Guid action, ref TrustData data);

        [DllImport("wintrust.dll", ExactSpelling = true)]
        private static extern IntPtr WTHelperProvDataFromStateData(IntPtr state);

        [DllImport("wintrust.dll", ExactSpelling = true)]
        private static extern IntPtr WTHelperGetProvSignerFromChain(
            IntPtr provider, uint index, [MarshalAs(UnmanagedType.Bool)] bool countersigner, uint counterIndex);

        [DllImport("wintrust.dll", ExactSpelling = true)]
        private static extern IntPtr WTHelperGetProvCertFromChain(IntPtr signer, uint index);
    }
}
